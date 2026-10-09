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
import dev.reins.android.design.ConfirmDialog
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

    fun signOut(device: DeviceView) {
        if (_ui.value.busy) return
        _ui.value = DevicesUi(busy = true)
        viewModelScope.launch {
            try {
                container.core.signOutDevice(device.id)
                container.feedback.play(Event.Revoked)
                _devices.value = container.core.devices()
                _ui.value = DevicesUi(message = "${device.name} is signed out. It can no longer open your vault or answer requests.")
            } catch (e: CancellationException) {
                throw e
            } catch (e: Exception) {
                container.feedback.play(Event.Error)
                _ui.value = DevicesUi(error = devicesError(e))
            }
        }
    }
}

/** Only the approval phone sees and signs out the account's devices. */
private fun devicesError(e: Exception): String =
    if (e is CoreException.Server && e.status.toInt() == 403) {
        "Only your approval phone lists and signs out devices. Use this phone for approvals first (Settings, Approval device)."
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
        ConfirmDialog(
            title = "Sign out ${untrusted(device.name)}?",
            text = "It can no longer open your vault, sync or answer requests. If you find it, sign in on it again.",
            confirmLabel = "Sign out",
            onConfirm = {
                signingOut = null
                viewModel.signOut(device)
            },
            onDismiss = { signingOut = null },
        )
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
