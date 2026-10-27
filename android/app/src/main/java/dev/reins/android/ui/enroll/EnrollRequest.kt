package dev.reins.android.ui.enroll

import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import androidx.lifecycle.ViewModel
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import androidx.lifecycle.viewModelScope
import dev.reins.android.AppContainer
import dev.reins.android.design.Banner
import dev.reins.android.design.BannerKind
import dev.reins.android.design.ButtonStyle
import dev.reins.android.design.Field
import dev.reins.android.design.Glyph
import dev.reins.android.design.GlyphIcon
import dev.reins.android.design.HoldButton
import dev.reins.android.design.LocalColors
import dev.reins.android.design.PaneRow
import dev.reins.android.design.RText
import dev.reins.android.design.RType
import dev.reins.android.design.SectionHeader
import dev.reins.android.design.SignalButton
import dev.reins.android.design.Spinner
import dev.reins.android.feedback.Event
import dev.reins.android.feedback.play
import dev.reins.android.platform.AuthResult
import dev.reins.android.platform.Authenticator
import dev.reins.android.ui.approval.AskDock
import dev.reins.android.ui.approval.AskField
import dev.reins.android.ui.approval.AskLoading
import dev.reins.android.ui.approval.FactGroup
import dev.reins.android.ui.approval.FactRow
import dev.reins.android.ui.common.quotedName
import dev.reins.android.ui.common.userMessage
import dev.reins.android.ui.join.spacedCode
import dev.reins.core.EnrollView
import kotlin.coroutines.cancellation.CancellationException
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.update
import kotlinx.coroutines.launch

data class EnrollRequestUi(
    val loading: Boolean = true,
    val view: EnrollView? = null,
    val busy: Boolean = false,
    val error: String? = null,
    /** Answered; [notice] says what an approval did. */
    val finished: Boolean = false,
    val notice: String? = null,
) {
    /** Approvable only once both phones show the code (the core refuses before), and not while answering. */
    val canApprove: Boolean get() = view?.code != null && !busy && !finished
}

/**
 * On the approving phone: another phone of the account asks to view it as a second phone (phone roles). The core's
 * `enroll_view` challenges it the first time and shows the code once the other phone revealed; the user compares the
 * two codes, holds Approve and passes the owner check, then `enroll_confirm` lists it as a viewer. Deny is one tap.
 * Nothing here is ever decided by Autopilot, a bypass or a grant: only this sheet answers it.
 */
class EnrollRequestViewModel(
    private val container: AppContainer,
    private val id: String,
    private val pollMillis: Long = 1_500L,
) : ViewModel() {
    private val _ui = MutableStateFlow(EnrollRequestUi())
    val ui: StateFlow<EnrollRequestUi> = _ui.asStateFlow()

    init {
        viewModelScope.launch {
            // Until the other phone shows its code (it reveals after this phone's challenge).
            while (true) {
                try {
                    val view = container.core.enrollView(id)
                    _ui.update { it.copy(loading = false, view = view, error = null) }
                    if (view.code != null) break
                } catch (e: CancellationException) {
                    throw e
                } catch (e: Exception) {
                    _ui.update { it.copy(loading = false, error = e.userMessage()) }
                    if (e !is dev.reins.core.CoreException.Network) break
                }
                delay(pollMillis)
            }
        }
    }

    fun approve(authenticator: Authenticator) {
        val current = _ui.value
        val view = current.view ?: return
        if (!current.canApprove) return
        _ui.update { it.copy(busy = true, error = null) }
        viewModelScope.launch {
            try {
                val device = quotedName(view.deviceName)
                when (authenticator.authenticate("Add a second phone", device)) {
                    AuthResult.Success -> {
                        container.core.enrollConfirm(id, true)
                        container.feedback.play(Event.Connected)
                        container.refreshAfterAnswer(id)
                        _ui.update { it.copy(busy = false, finished = true, notice = "$device sees this account now. It decides nothing.") }
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
        if (_ui.value.busy || _ui.value.finished) return
        _ui.update { it.copy(busy = true, error = null) }
        container.feedback.play(Event.Denied)
        viewModelScope.launch {
            try {
                container.core.enrollConfirm(id, false)
                container.refreshAfterAnswer(id)
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

/**
 * "Add a second phone" (Next, Pair and join 04): the accounts field with the code the other phone shows, large; its
 * name quoted on a row of its own (it chose it; the code covers its keys, not its name); what it gets; Deny, and
 * Approve held, then the owner check.
 */
@Composable
fun EnrollRequestSheet(viewModel: EnrollRequestViewModel, authenticator: Authenticator, onDone: (String?) -> Unit, onClose: () -> Unit = { onDone(null) }) {
    val c = LocalColors.current
    val ui by viewModel.ui.collectAsStateWithLifecycle()
    LaunchedEffect(ui.finished) { if (ui.finished) onDone(ui.notice) }
    val view = ui.view
    if (view == null) {
        AskLoading(ui.loading, ui.error, onClose, errorTag = "enrollError")
        return
    }
    val device = quotedName(view.deviceName)
    Column(Modifier.fillMaxSize().background(c.paper).testTag("enrollRequest")) {
        Column(Modifier.weight(1f).verticalScroll(rememberScrollState())) {
            AskField(
                Field.Accounts,
                "Account · Second phone",
                "Add a second phone",
                "phone",
                onClose = onClose,
                askTag = "enrollTitle",
                closeGlyph = Glyph.Close,
                closeLabel = "Close",
                top = {
                    GlyphIcon(Glyph.Phone, c.onField, size = 20.dp)
                    RText("Second phone", RType.sans(14.5f, FontWeight.Medium), c.onField, Modifier.weight(1f), maxLines = 1)
                },
                verdict = null,
                below = {
                    Column(Modifier.padding(top = 14.dp)) {
                        val code = view.code
                        if (code == null) {
                            Row(horizontalArrangement = Arrangement.spacedBy(10.dp), verticalAlignment = Alignment.CenterVertically) {
                                Spinner(c.onField, size = 20.dp)
                                RText("Waiting for its code…", RType.sans(15f, FontWeight.Medium), c.onField, Modifier.testTag("enrollWaiting"), maxLines = 1)
                            }
                        } else {
                            RText(spacedCode(code), RType.codes.copy(fontSize = 48.sp, letterSpacing = 3.sp), c.onField, Modifier.testTag("enrollCode"), maxLines = 1, ltr = true)
                            RText("Approve only if it shows this code.", RType.sans(14f, FontWeight.Medium), c.onField2, Modifier.padding(top = 6.dp), maxLines = 2)
                        }
                    }
                },
            )
            FactGroup(Modifier.padding(top = 14.dp), tag = "enrollWho", rows = listOf({ FactRow("Calls itself", device, valueTag = "enrollDevice") }))
            SectionHeader("It gets", quiet = true)
            FactGroup(
                rows = listOf(
                    { PaneRow("Sees connections and activity", leading = { GlyphIcon(Glyph.Eye, c.inkMeta, size = 18.dp) }, first = true, flat = true) },
                    { PaneRow("Decides nothing", leading = { GlyphIcon(Glyph.Block, c.inkMeta, size = 18.dp) }, first = true, flat = true) },
                ),
            )
            ui.error?.let { Banner(it, Modifier.padding(horizontal = 16.dp).padding(top = 16.dp), BannerKind.Error, tag = "enrollError") }
        }
        AskDock {
            Row(Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.spacedBy(10.dp)) {
                SignalButton("Deny", Modifier.weight(1f).testTag("deny"), style = ButtonStyle.Outline, enabled = !ui.busy, onClick = viewModel::deny)
                HoldButton(
                    "Hold to approve",
                    c.ink,
                    Modifier.weight(1.35f).testTag("approve"),
                    enabled = ui.canApprove,
                    busy = ui.busy,
                    onConfirm = { viewModel.approve(authenticator) },
                )
            }
        }
    }
}

/**
 * A `Join` item: a phone asking to view the account (an enrollment, [EnrollRequestSheet]) or one asking for its keys
 * (an account without phone roles, [legacy]). The core says which.
 */
class JoinKindViewModel(container: AppContainer, id: String) : ViewModel() {
    private val _enrollment = MutableStateFlow<Boolean?>(null)
    val enrollment: StateFlow<Boolean?> = _enrollment.asStateFlow()

    init {
        viewModelScope.launch {
            _enrollment.value = try {
                container.core.joinIsEnrollment(id)
            } catch (e: CancellationException) {
                throw e
            } catch (e: Exception) {
                // Gone or unreadable: the legacy sheet says so as it always did.
                false
            }
        }
    }
}
