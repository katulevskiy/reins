package dev.reins.android.ui.enroll

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import androidx.lifecycle.ViewModel
import androidx.lifecycle.viewModelScope
import dev.reins.android.AppContainer
import dev.reins.android.design.ButtonStyle
import dev.reins.android.design.LocalColors
import dev.reins.android.design.RText
import dev.reins.android.design.RType
import dev.reins.android.design.SignalButton
import dev.reins.android.design.Spinner
import dev.reins.android.feedback.Event
import dev.reins.android.feedback.play
import dev.reins.android.ui.common.userMessage
import dev.reins.android.ui.join.spacedCode
import dev.reins.core.CoreException
import dev.reins.core.EnrollProgress
import kotlin.coroutines.cancellation.CancellationException
import kotlinx.coroutines.Job
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.launch

/**
 * "Join as a second phone", on a phone that is signed in but neither approves nor views (phone roles): the core's
 * viewer enrollment (`enroll_begin`, then `enroll_poll`), seen as steps. The approving phone decides; this phone only
 * shows the code to compare.
 */
sealed interface ViewerJoinStep {
    /** Nothing asked yet. */
    data object Idle : ViewerJoinStep

    /** Asking the server. */
    data object Starting : ViewerJoinStep

    /** Asked; the approving phone has not answered yet. */
    data object Waiting : ViewerJoinStep

    /** The code to compare with the approving phone's ("482 193"). */
    data class Compare(val code: String) : ViewerJoinStep

    data object Enrolled : ViewerJoinStep

    data object Denied : ViewerJoinStep

    data object Expired : ViewerJoinStep

    data class Failed(val message: String) : ViewerJoinStep
}

/** Still waiting on the approving phone: keep polling. */
val ViewerJoinStep.polling: Boolean get() = this is ViewerJoinStep.Waiting || this is ViewerJoinStep.Compare

/** Over: Try again starts a new request (none after an enrollment). */
val ViewerJoinStep.ended: Boolean
    get() = this is ViewerJoinStep.Denied || this is ViewerJoinStep.Expired || this is ViewerJoinStep.Failed

/**
 * The step after a poll. A code once shown is never taken back by a late "waiting" (the core keeps the first
 * challenge and refuses another); the end states are final.
 */
fun ViewerJoinStep.after(progress: EnrollProgress): ViewerJoinStep = when (progress) {
    is EnrollProgress.Waiting -> if (this is ViewerJoinStep.Compare) this else ViewerJoinStep.Waiting
    is EnrollProgress.Compare -> ViewerJoinStep.Compare(spacedCode(progress.code))
    is EnrollProgress.Enrolled -> ViewerJoinStep.Enrolled
    is EnrollProgress.Denied -> ViewerJoinStep.Denied
    is EnrollProgress.Expired -> ViewerJoinStep.Expired
}

/** A poll that failed: a lost connection keeps waiting, an enrollment the core no longer has is over. */
fun ViewerJoinStep.afterFailure(e: Exception): ViewerJoinStep = when (e) {
    is CoreException.Network -> this
    is CoreException.NotFound -> ViewerJoinStep.Expired
    else -> ViewerJoinStep.Failed(e.userMessage())
}

class ViewerJoinViewModel(
    private val container: AppContainer,
    private val deviceName: String,
    private val pollMillis: Long = POLL_MILLIS,
) : ViewModel() {
    private val _step = MutableStateFlow<ViewerJoinStep>(ViewerJoinStep.Idle)
    val step: StateFlow<ViewerJoinStep> = _step.asStateFlow()
    private var job: Job? = null

    /** Asks the approving phone (again, after an end), then follows the answer. */
    fun start() {
        val now = _step.value
        if (now == ViewerJoinStep.Starting || now.polling || now == ViewerJoinStep.Enrolled) return
        _step.value = ViewerJoinStep.Starting
        job?.cancel()
        job = viewModelScope.launch {
            try {
                container.core.enrollBegin(deviceName)
                _step.value = ViewerJoinStep.Waiting
            } catch (e: CancellationException) {
                throw e
            } catch (e: Exception) {
                container.feedback.play(Event.Error)
                _step.value = ViewerJoinStep.Failed(e.userMessage())
                return@launch
            }
            while (_step.value.polling) {
                delay(pollMillis)
                val next = try {
                    _step.value.after(container.core.enrollPoll())
                } catch (e: CancellationException) {
                    throw e
                } catch (e: Exception) {
                    _step.value.afterFailure(e)
                }
                if (next is ViewerJoinStep.Compare && _step.value !is ViewerJoinStep.Compare) container.feedback.play(Event.Alert)
                _step.value = next
            }
            when (_step.value) {
                ViewerJoinStep.Enrolled -> {
                    container.feedback.play(Event.Connected)
                    // A viewer now: what the server keeps for the account's phones loads.
                    container.refreshConnections()
                }
                ViewerJoinStep.Denied -> container.feedback.play(Event.Denied)
                else -> Unit
            }
        }
    }

    /** Closed before the end: the request is withdrawn (the approving phone can no longer approve it). */
    fun cancel() {
        val asked = _step.value.polling
        job?.cancel()
        job = null
        _step.value = ViewerJoinStep.Idle
        if (asked) {
            container.appScope.launch {
                try {
                    container.core.enrollCancel()
                } catch (e: CancellationException) {
                    throw e
                } catch (e: Exception) {
                    // It expires on the server anyway.
                }
            }
        }
    }

    override fun onCleared() {
        if (_step.value.polling) cancel()
    }

    companion object {
        const val POLL_MILLIS = 2_000L
    }
}

/** What the sheet shows for each step: short, as on the boards (Pair and join). */
@Composable
fun ViewerJoinContent(step: ViewerJoinStep, onStart: () -> Unit, onDone: () -> Unit) {
    val c = LocalColors.current
    Column(
        Modifier.fillMaxWidth().padding(horizontal = 24.dp).padding(top = 4.dp, bottom = 12.dp).testTag("viewerJoin"),
        verticalArrangement = Arrangement.spacedBy(14.dp),
        horizontalAlignment = Alignment.Start,
    ) {
        when (step) {
            ViewerJoinStep.Idle -> {
                RText(VIEWER_JOIN_INTRO, RType.sans(15f, lineHeight = 21f), c.ink2)
                SignalButton("Ask the approving phone", Modifier.fillMaxWidth().testTag("viewerJoinStart"), onClick = onStart)
            }
            ViewerJoinStep.Starting, ViewerJoinStep.Waiting -> {
                Spinner(c.inkMeta, size = 28.dp)
                RText("Open Reins on the approving phone.", RType.sans(15f, lineHeight = 21f), c.ink2, Modifier.testTag("viewerJoinWaiting"))
            }
            is ViewerJoinStep.Compare -> {
                RText(step.code, RType.codes.copy(fontSize = 44.sp, letterSpacing = 3.sp), c.ink, Modifier.testTag("viewerJoinCode"), maxLines = 1, ltr = true)
                RText("Approve there only if it shows this code.", RType.sans(15f, FontWeight.Medium), c.ink2)
            }
            ViewerJoinStep.Enrolled -> {
                RText("This phone sees your connections now.", RType.sans(15f, lineHeight = 21f), c.ink, Modifier.testTag("viewerJoinDone"))
                SignalButton("Done", Modifier.fillMaxWidth().testTag("viewerJoinClose"), onClick = onDone)
            }
            ViewerJoinStep.Denied, ViewerJoinStep.Expired, is ViewerJoinStep.Failed -> {
                val text = when (step) {
                    ViewerJoinStep.Denied -> "The approving phone said no."
                    ViewerJoinStep.Expired -> "The request ran out."
                    else -> (step as ViewerJoinStep.Failed).message
                }
                RText(text, RType.sans(15f, lineHeight = 21f), c.ink2, Modifier.testTag("viewerJoinEnded"))
                SignalButton("Try again", Modifier.fillMaxWidth().testTag("viewerJoinRetry"), style = ButtonStyle.Outline, onClick = onStart)
            }
        }
    }
}

/** What joining does, before asking. */
const val VIEWER_JOIN_INTRO =
    "Your approving phone adds this one as a second phone. It sees your connections and activity; it decides nothing."
