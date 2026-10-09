package dev.reins.android.ui.settings

import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.width
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import androidx.lifecycle.ViewModel
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import androidx.lifecycle.viewModelScope
import dev.reins.android.AppContainer
import dev.reins.android.design.Banner
import dev.reins.android.design.BannerKind
import dev.reins.android.design.ButtonStyle
import dev.reins.android.design.CapsuleButton
import dev.reins.android.design.Glyph
import dev.reins.android.design.GlyphIcon
import dev.reins.android.design.Group
import dev.reins.android.design.Hairline
import dev.reins.android.design.ListRow
import dev.reins.android.design.LocalColors
import dev.reins.android.design.RText
import dev.reins.android.design.RType
import dev.reins.android.design.Screen
import dev.reins.android.design.Tag
import dev.reins.android.design.glass
import dev.reins.android.feedback.Event
import dev.reins.android.feedback.play
import dev.reins.android.state.AppState
import dev.reins.android.ui.common.relativeTime
import dev.reins.android.ui.common.untrusted
import dev.reins.android.ui.common.userMessage
import dev.reins.core.CoreException
import dev.reins.core.DeviceKind
import dev.reins.core.DeviceView
import kotlin.coroutines.cancellation.CancellationException
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.launch

data class DevicesUi(val busy: Boolean = false, val error: String? = null, val message: String? = null)

/** Settings > Devices: the account's devices, and signing one out (a lost phone). */
class DevicesViewModel(private val container: AppContainer) : ViewModel() {
    /** Null until first read. */
    private val _devices = MutableStateFlow<List<DeviceView>?>(null)
    val devices: StateFlow<List<DeviceView>?> = _devices.asStateFlow()

    /** The eight digits of this phone's key, which `reins vault add` asks for once. */
    private val _phoneKey = MutableStateFlow<String?>(null)
    val phoneKey: StateFlow<String?> = _phoneKey.asStateFlow()

    private val _ui = MutableStateFlow(DevicesUi())
    val ui: StateFlow<DevicesUi> = _ui.asStateFlow()

    fun load() {
        viewModelScope.launch {
            try {
                _devices.value = container.core.devices()
            } catch (e: CancellationException) {
                throw e
            } catch (e: Exception) {
                _ui.value = DevicesUi(error = devicesError(e))
            }
        }
        viewModelScope.launch {
            _phoneKey.value = try {
                container.core.phoneKeyFingerprint()
            } catch (e: CancellationException) {
                throw e
            } catch (e: Exception) {
                null
            }
        }
    }

    /** Signs [device] out with the recovery code or master password the user typed now (never one the phone keeps). */
    fun signOut(device: DeviceView, codeOrPassword: String, onDone: (Boolean) -> Unit = {}) {
        if (_ui.value.busy) return
        _ui.value = DevicesUi(busy = true)
        viewModelScope.launch {
            try {
                container.core.signOutDevice(device.id, codeOrPassword)
                container.feedback.play(Event.Revoked)
                _devices.value = container.core.devices()
                _ui.value = DevicesUi(
                    message = "${device.name} is signed out. It can no longer open your vault or answer requests, and " +
                        "cannot sign in again as it is.",
                )
                onDone(true)
            } catch (e: CancellationException) {
                throw e
            } catch (e: Exception) {
                container.feedback.play(Event.Error)
                _ui.value = DevicesUi(error = devicesError(e))
                onDone(false)
            }
        }
    }
}

/** Only the approval phone sees and signs out the account's devices. */
private fun devicesError(e: Exception): String =
    if (e is CoreException.Server && e.status.toInt() == 403) {
        "Devices are listed and signed out from your approval phone, the one that receives the requests."
    } else {
        e.userMessage()
    }

fun deviceGlyph(kind: DeviceKind): Glyph = when (kind) {
    DeviceKind.PHONE -> Glyph.Phone
    DeviceKind.COMPUTER -> Glyph.Laptop
    DeviceKind.BROWSER -> Glyph.Link
    DeviceKind.OTHER -> Glyph.Apps
}

@Composable
fun DevicesScreen(
    viewModel: DevicesViewModel,
    state: AppState,
    onBack: () -> Unit,
    onConnection: (String) -> Unit,
) {
    val c = LocalColors.current
    val devices by viewModel.devices.collectAsStateWithLifecycle()
    val phoneKey by viewModel.phoneKey.collectAsStateWithLifecycle()
    val ui by viewModel.ui.collectAsStateWithLifecycle()
    val connections by state.connections.collectAsStateWithLifecycle()
    var signingOut by remember { mutableStateOf<DeviceView?>(null) }
    LaunchedEffect(viewModel) { viewModel.load() }
    // This phone's key digits, and the recovery code typed to sign a phone out.
    dev.reins.android.ui.common.SecureWindow()

    Screen(title = "Devices", onBack = onBack) {
        Column(Modifier.padding(horizontal = 16.dp)) {
            ui.message?.let { Banner(untrusted(it), Modifier.padding(top = 10.dp), tag = "devicesMessage") }
            ui.error?.let { Banner(it, Modifier.padding(top = 10.dp), BannerKind.Error, tag = "devicesError") }
        }
        val list = devices
        val me = list?.firstOrNull { it.thisDevice }
        Group(header = "This phone") {
            ListRow(
                me?.name?.let(::untrusted) ?: "This phone",
                Modifier.testTag("thisDevice"),
                subtitle = listOfNotNull(
                    if (me?.approval == true) "Your approval device: requests come here" else null,
                    phoneKey?.let { "Key for reins vault add: $it" },
                ).joinToString("\n").ifEmpty { null },
                glyph = Glyph.Phone,
                tint = c.accent,
            )
        }
        val others = list.orEmpty().filterNot { it.thisDevice }
        Group(
            header = "Other phones and apps",
            footer = "Lost a phone? Sign it out here. It can no longer open your vault or answer requests; " +
                "what it kept stays encrypted behind its screen lock.",
        ) {
            when {
                list == null && ui.error == null -> RText("Loading…", RType.sans(15f), c.secondary, Modifier.padding(16.dp))
                others.isEmpty() -> RText(
                    "No other device is signed in to your account.",
                    RType.sans(15f),
                    c.secondary,
                    Modifier.padding(16.dp).testTag("noOtherDevices"),
                )
                else -> others.forEachIndexed { i, device ->
                    if (i > 0) Hairline(inset = 51.dp)
                    DeviceRow(device, busy = ui.busy) { signingOut = device }
                }
            }
        }
        val computers = connections.filter { it.keyFingerprint != null }
        Group(
            header = "Computers",
            footer = "A computer's page removes it; on the computer, reins logout does the same.",
        ) {
            if (computers.isEmpty()) {
                RText("No computer is connected.", RType.sans(15f), c.secondary, Modifier.padding(16.dp).testTag("noComputers"))
            }
            computers.forEachIndexed { i, connection ->
                if (i > 0) Hairline(inset = 51.dp)
                ListRow(
                    untrusted(connection.label),
                    Modifier.testTag("deviceComputer:${connection.id}"),
                    subtitle = listOfNotNull(
                        connection.keyFingerprint?.let { "Key $it" },
                        connection.lastUsedAt?.let { "used ${relativeTime(it)}" } ?: "never used",
                    ).joinToString(" · "),
                    glyph = Glyph.Laptop,
                    chevron = true,
                    onClick = { onConnection(connection.id) },
                )
            }
        }
        Spacer(Modifier.padding(bottom = 32.dp))
    }

    signingOut?.let { device ->
        SignOutDialog(
            device = device,
            busy = ui.busy,
            error = ui.error,
            onConfirm = { typed -> viewModel.signOut(device, typed) { ok -> if (ok) signingOut = null } },
            onDismiss = { signingOut = null },
        )
    }
}

/**
 * "Sign out <device>?": what it is (two phones may share a name), what signing out does, and the recovery code or master
 * password typed now, the proof the server asks for.
 */
@Composable
private fun SignOutDialog(device: DeviceView, busy: Boolean, error: String?, onConfirm: (String) -> Unit, onDismiss: () -> Unit) {
    val c = LocalColors.current
    var typed by remember { mutableStateOf("") }
    androidx.compose.ui.window.Dialog(
        onDismissRequest = onDismiss,
        properties = androidx.compose.ui.window.DialogProperties(usePlatformDefaultWidth = false),
    ) {
        dev.reins.android.feedback.DialogFeedback()
        Column(
            Modifier
                .padding(horizontal = 28.dp)
                .fillMaxWidth()
                .glass(c, androidx.compose.foundation.shape.RoundedCornerShape(26.dp), 16.dp)
                .padding(22.dp)
                .testTag("signOutDialog"),
        ) {
            RText("Sign out ${untrusted(device.name)}?", RType.sans(19f, FontWeight.SemiBold), c.text)
            RText(
                "${device.platform} · signed in ${relativeTime(device.createdAt)} · last seen ${relativeTime(device.lastSeenAt)}",
                RType.sans(13.5f),
                c.secondary,
                Modifier.padding(top = 4.dp),
            )
            RText(
                "It can no longer open your vault, sync or answer requests, and cannot sign in again as it is. " +
                    "Type your recovery code (or master password) to confirm.",
                RType.sans(15f, lineHeight = 21f),
                c.secondary,
                Modifier.padding(top = 10.dp),
            )
            dev.reins.android.design.RTextField(
                typed,
                { typed = it },
                "Recovery code or master password",
                Modifier.padding(top = 14.dp),
                tag = "signOutProof",
                enabled = !busy,
                mono = true,
                password = true,
                keyboardOptions = androidx.compose.foundation.text.KeyboardOptions(
                    keyboardType = androidx.compose.ui.text.input.KeyboardType.Password,
                    autoCorrectEnabled = false,
                ),
            )
            error?.let { Banner(it, Modifier.padding(top = 10.dp), BannerKind.Error, tag = "signOutError") }
            Row(Modifier.padding(top = 18.dp), horizontalArrangement = androidx.compose.foundation.layout.Arrangement.spacedBy(10.dp)) {
                CapsuleButton("Cancel", Modifier.weight(1f), style = ButtonStyle.Secondary, onClick = onDismiss)
                CapsuleButton(
                    "Sign out",
                    Modifier.weight(1f).testTag("confirmSignOut"),
                    style = ButtonStyle.Destructive,
                    enabled = typed.isNotBlank(),
                    busy = busy,
                ) { onConfirm(typed) }
            }
        }
    }
}

@Composable
private fun DeviceRow(device: DeviceView, busy: Boolean, onSignOut: () -> Unit) {
    val c = LocalColors.current
    Row(
        Modifier.fillMaxWidth().testTag("device:${device.id}").padding(start = 16.dp, end = 12.dp, top = 10.dp, bottom = 10.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        GlyphIcon(deviceGlyph(device.kind), c.secondary, size = 21.dp)
        Spacer(Modifier.width(14.dp))
        Column(Modifier.weight(1f)) {
            RText(untrusted(device.name), RType.sans(16f, FontWeight.Medium), c.text, maxLines = 1)
            RText(
                "${device.platform} · last seen ${relativeTime(device.lastSeenAt)}",
                RType.sans(13f),
                c.secondary,
                Modifier.padding(top = 2.dp),
                maxLines = 2,
            )
            if (device.approval) Tag("Approval device", Modifier.padding(top = 4.dp), tint = c.accent)
        }
        Spacer(Modifier.width(8.dp))
        CapsuleButton(
            "Sign out",
            Modifier.testTag("signOutDevice:${device.id}"),
            style = ButtonStyle.Destructive,
            compact = true,
            enabled = !busy,
            onClick = onSignOut,
        )
    }
}
