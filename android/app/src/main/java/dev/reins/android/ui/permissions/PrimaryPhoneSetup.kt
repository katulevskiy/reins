package dev.reins.android.ui.permissions

import android.os.Build
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.runtime.*
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.unit.dp
import dev.reins.android.AppContainer
import dev.reins.android.design.*
import dev.reins.android.platform.AuthResult
import dev.reins.android.platform.Authenticator
import dev.reins.android.platform.OwnerPrompt
import dev.reins.android.ui.common.SecureWindow
import dev.reins.android.ui.common.untrusted
import dev.reins.android.ui.common.userMessage
import dev.reins.core.PhoneRole
import dev.reins.core.RosterSummary
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.launch

data class PrimaryPhoneSetupUi(
    val checking: Boolean = true,
    val busy: Boolean = false,
    val role: PhoneRole? = null,
    val hasRoster: Boolean = false,
    val primaryName: String? = null,
    val error: String? = null,
    val complete: Boolean = false,
) {
    val canCreate: Boolean get() = !checking && !busy && role == PhoneRole.LEGACY && !hasRoster
}

/** Explicit account-wide role creation. Checking this screen never creates or recovers authority. */
class PrimaryPhoneSetupModel(
    private val container: AppContainer,
    private val scope: CoroutineScope,
    private val parentCurrent: () -> Boolean,
) {
    private val epoch = container.state.accountEpoch.value
    private val _ui = MutableStateFlow(PrimaryPhoneSetupUi())
    val ui = _ui.asStateFlow()
    private var visible = true
    private var revision = 0L
    private fun current(token: Long) = visible && revision == token && parentCurrent() && container.state.isCurrent(epoch)
    fun deactivate() { visible = false; revision++ }
    fun cancelAttempt() { revision++; _ui.value = _ui.value.copy(busy = false) }

    fun check() {
        val token = revision
        if (!current(token) || _ui.value.busy) return
        _ui.value = _ui.value.copy(checking = true, error = null)
        scope.launch {
            try {
                val roster = container.core.rosterSummary(true)
                val role = container.core.phoneRole()
                if (current(token)) _ui.value = checked(roster, role)
            } catch (e: CancellationException) { throw e } catch (e: Exception) {
                if (current(token)) _ui.value = PrimaryPhoneSetupUi(checking = false, error = e.userMessage())
            }
        }
    }

    fun enable(backupConfirmed: Boolean, proof: String?, authenticator: Authenticator, onPrimary: () -> Unit) {
        val token = revision
        if (!current(token) || !backupConfirmed || !_ui.value.canCreate) return
        _ui.value = _ui.value.copy(busy = true, error = null)
        scope.launch {
            try {
                // The bound core API captures its session before invoking this original-attempt owner callback.
                val owner = OwnerPrompt(Authenticator { title, reason ->
                    if (!current(token)) AuthResult.Cancelled else {
                        val answer = authenticator.authenticate(title, reason)
                        if (current(token)) answer else AuthResult.Cancelled
                    }
                }, "Enable automatic permissions")
                val roster = container.core.rosterSummary(true)
                val role = container.core.phoneRole()
                if (!current(token)) return@launch
                if (roster != null || role != PhoneRole.LEGACY) {
                    val confirmed = role == PhoneRole.PRIMARY && roster?.members?.any { it.primary && it.thisPhone } == true
                    _ui.value = checked(roster, role).copy(complete = confirmed)
                    if (confirmed) onPrimary()
                    return@launch
                }
                val created = try {
                    // Core rechecks the server/local roster after owner confirmation, before creating authority.
                    container.core.rosterCreateAuthorized(Build.MODEL, proof?.takeIf { it.isNotEmpty() }, owner)
                } catch (e: CancellationException) { throw e } catch (e: Exception) {
                    when (owner.last) {
                        AuthResult.Cancelled -> return@launch
                        AuthResult.Unavailable -> { if (current(token)) _ui.value = _ui.value.copy(error = "Set a screen lock or fingerprint on this phone first."); return@launch }
                        else -> throw e
                    }
                }
                if (!current(token)) return@launch
                if (created.role != PhoneRole.PRIMARY || created.members.none { it.primary && it.thisPhone }) {
                    _ui.value = checked(created, created.role).copy(error = "This phone was not confirmed as the primary. Reload its status before trying again.")
                    return@launch
                }
                _ui.value = checked(created, created.role).copy(complete = true)
                onPrimary()
            } catch (e: CancellationException) { throw e } catch (e: Exception) {
                if (current(token)) _ui.value = _ui.value.copy(error = e.userMessage())
            } finally { if (current(token)) _ui.value = _ui.value.copy(busy = false) }
        }
    }

    private fun checked(roster: RosterSummary?, role: PhoneRole) = PrimaryPhoneSetupUi(
        checking = false, role = role, hasRoster = roster != null,
        primaryName = roster?.members?.firstOrNull { it.primary }?.name,
    )
}

@Composable
fun PrimaryPhoneSetupSection(model: AccountPermissionsViewModel, authenticator: Authenticator) {
    val setup = remember(model) { model.primaryPhoneSetup() }
    val state by setup.ui.collectAsState()
    val c = LocalColors.current
    var showing by remember(setup) { mutableStateOf(false) }
    var backupConfirmed by remember(setup) { mutableStateOf(false) }
    var enterProof by remember(setup) { mutableStateOf(false) }
    var proof by remember(setup) { mutableStateOf("") }
    fun close() { proof = ""; enterProof = false; backupConfirmed = false; showing = false; setup.cancelAttempt() }
    DisposableEffect(setup) { onDispose { proof = ""; setup.deactivate() } }
    LaunchedEffect(setup) { setup.check() }
    Group(header = "Automatic permissions on this phone") {
        Column(Modifier.padding(16.dp), verticalArrangement = Arrangement.spacedBy(12.dp)) {
            when {
                state.checking -> RText("Checking this phone…", RType.sans(14f), c.ink2)
                state.canCreate -> {
                    RText("A verified primary phone is required before automatic permissions can be enabled.", RType.sans(14f), c.ink2)
                    CapsuleButton("Enable automatic permissions on this phone", Modifier.fillMaxWidth().testTag("primarySetupOpen"), maxLines = Int.MAX_VALUE) { showing = true }
                }
                state.role == PhoneRole.PRIMARY -> {
                    RText("This phone is already the primary. Current policy and history requirements still apply.", RType.sans(14f), c.ink2)
                    CapsuleButton("Reload permissions", Modifier.fillMaxWidth().testTag("primarySetupReload"), style = ButtonStyle.Soft, maxLines = Int.MAX_VALUE, onClick = model::reload)
                }
                state.role == PhoneRole.VIEWER || state.role == PhoneRole.REMOVED || state.hasRoster -> RText("Use your primary phone${state.primaryName?.let { " (${untrusted(it)})" }.orEmpty()} to manage automatic permissions. This setup cannot take over another phone's role.", RType.sans(14f), c.ink2, Modifier.testTag("primarySetupGuidance"))
            }
            if (!showing) state.error?.let {
                Banner(untrusted(it), kind = BannerKind.Error, tag = "primarySetupError")
                CapsuleButton("Check again", Modifier.fillMaxWidth(), style = ButtonStyle.Soft, maxLines = Int.MAX_VALUE, onClick = setup::check)
            }
        }
    }
    if (showing) {
        SecureWindow()
        androidx.compose.ui.window.Dialog(onDismissRequest = ::close, properties = androidx.compose.ui.window.DialogProperties(usePlatformDefaultWidth = false, securePolicy = if (dev.reins.android.BuildConfig.SECURE_SCREENS) androidx.compose.ui.window.SecureFlagPolicy.SecureOn else androidx.compose.ui.window.SecureFlagPolicy.Inherit)) {
            Screen("Enable automatic permissions", onBack = ::close) {
                Column(Modifier.padding(16.dp).testTag("primarySetupDialog"), verticalArrangement = Arrangement.spacedBy(12.dp)) {
                    RText("This makes this phone the primary approval phone for your whole Reins account, including its other integrations. It creates protected recovery information. It does not allow any agent action by itself.", RType.sans(15f), c.ink)
                    CheckRow(backupConfirmed, { backupConfirmed = it }, enabled = !state.busy, tag = "primarySetupBackup") { RText("I have saved my recovery code or master password.", RType.sans(15f), c.ink) }
                    if (enterProof) RTextField(proof, { proof = it }, "Recovery code or master password", password = true, keyboardOptions = KeyboardOptions(autoCorrectEnabled = false, keyboardType = KeyboardType.Password), enabled = !state.busy, tag = "primarySetupProof")
                    else CapsuleButton("Use a recovery code or master password", Modifier.fillMaxWidth(), enabled = !state.busy, style = ButtonStyle.Soft, maxLines = Int.MAX_VALUE) { enterProof = true }
                    state.error?.let { Banner(untrusted(it), kind = BannerKind.Error, tag = "primarySetupError") }
                    if (!state.canCreate && !state.busy) RText("The phone's role changed. Close this screen and check its current status.", RType.sans(14f), c.ink2)
                    CapsuleButton("Make this my primary phone", Modifier.fillMaxWidth().testTag("primarySetupConfirm"), enabled = backupConfirmed && state.canCreate, busy = state.busy, maxLines = Int.MAX_VALUE) {
                        val entered = proof; proof = ""
                        setup.enable(backupConfirmed, entered, authenticator) { close(); model.reload() }
                    }
                    CapsuleButton("Cancel", Modifier.fillMaxWidth().testTag("primarySetupCancel"), style = ButtonStyle.Soft, maxLines = Int.MAX_VALUE, onClick = ::close)
                }
            }
        }
    }
}
