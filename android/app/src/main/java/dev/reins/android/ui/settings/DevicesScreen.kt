package dev.reins.android.ui.settings

import dev.reins.android.design.pressable
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.imePadding
import androidx.compose.foundation.layout.navigationBars
import androidx.compose.foundation.layout.windowInsetsPadding
import androidx.compose.ui.draw.clip
import androidx.compose.ui.unit.sp
import dev.reins.android.design.hairlineBelow

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
import dev.reins.core.PhoneRole
import dev.reins.core.RosterMemberView
import dev.reins.core.RosterSummary
import dev.reins.android.ui.common.untrustedName
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

    /** The account's phones (phone roles) as this phone verified them; null without a roster. */
    private val _roster = MutableStateFlow<RosterSummary?>(null)
    val roster: StateFlow<RosterSummary?> = _roster.asStateFlow()

    private fun loadRoster() {
        viewModelScope.launch {
            _roster.value = try {
                container.core.rosterSummary(true)
            } catch (e: CancellationException) {
                throw e
            } catch (e: Exception) {
                null
            }
        }
    }

    /** Takes a second phone off the account (held to confirm): it reads nothing written from now on. */
    fun removeViewer(member: RosterMemberView, onDone: (Boolean) -> Unit = {}) {
        if (_ui.value.busy || member.primary || member.thisPhone) return
        _ui.value = DevicesUi(busy = true)
        viewModelScope.launch {
            try {
                container.core.rosterRemoveMember(member.deviceId)
                container.feedback.play(Event.Revoked)
                _roster.value = container.core.rosterSummary(false)
                _ui.value = DevicesUi(message = "${untrustedName(member.name)} no longer sees this account.")
                onDone(true)
            } catch (e: CancellationException) {
                throw e
            } catch (e: Exception) {
                container.feedback.play(Event.Error)
                _ui.value = DevicesUi(error = e.userMessage())
                onDone(false)
            }
        }
    }

    fun load() {
        loadRoster()
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
    val roster by viewModel.roster.collectAsStateWithLifecycle()
    var removing by remember { mutableStateOf<RosterMemberView?>(null) }
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
                glyph = Glyph.Phone,
                trailing = if (me?.approval == true) ({ dev.reins.android.design.NextPill("Approves", c.success, glyph = Glyph.ShieldCheck) }) else null,
            )
            phoneKey?.let { key ->
                Hairline()
                ListRow("Key", Modifier.testTag("thisDeviceKey"), glyph = Glyph.Terminal, trailing = { RText(key, RType.mono(13f), c.ink, maxLines = 1, ltr = true) })
            }
        }
        val others = list.orEmpty().filterNot { it.thisDevice }
        Group(
            header = "Other phones and apps",
            footer = "Lost a phone? Sign it out here: the app on it can no longer open your vault or answer requests, " +
                "nor sign in again as it is. Whoever has it and its screen lock could still read your recovery code " +
                "on it and sign in with a new install; reset the vault if that may happen.",
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
        // Phone roles, on the approving phone: the second phones (viewers) it added, each removable (held).
        roster?.takeIf { it.role == PhoneRole.PRIMARY }?.let { summary ->
            val viewers = summary.members.filter { !it.primary }
            Group(header = "Second phones", footer = "They see your connections and activity. They decide nothing.") {
                if (viewers.isEmpty()) {
                    RText("No second phone.", RType.sans(15f), c.secondary, Modifier.padding(16.dp).testTag("noViewers"))
                }
                viewers.forEachIndexed { i, member ->
                    if (i > 0) Hairline(inset = 51.dp)
                    ViewerRow(member, busy = ui.busy) { removing = member }
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

    removing?.let { member ->
        RemoveViewerDialog(
            member = member,
            busy = ui.busy,
            error = ui.error,
            onConfirm = { viewModel.removeViewer(member) { ok -> if (ok) removing = null } },
            onDismiss = { removing = null },
        )
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
        var help by remember { mutableStateOf(false) }
        // A sheet from the bottom (Signal): the question in the serif, the phone on its own row, the proof, the action.
        androidx.compose.foundation.layout.Box(Modifier.fillMaxSize(), contentAlignment = Alignment.BottomCenter) {
            val shape = androidx.compose.foundation.shape.RoundedCornerShape(topStart = 28.dp, topEnd = 28.dp)
            Column(
                Modifier
                    .fillMaxWidth()
                    .clip(shape)
                    .background(c.paperRaised)
                    .windowInsetsPadding(androidx.compose.foundation.layout.WindowInsets.navigationBars)
                    .imePadding()
                    .padding(start = 22.dp, end = 22.dp, top = 22.dp, bottom = 16.dp)
                    .testTag("signOutDialog"),
            ) {
                dev.reins.android.design.AskText("Sign a phone out", c.ink, target = "out", style = RType.serif(32f, 36f), minSize = 26.sp)
                Row(Modifier.padding(top = 12.dp).fillMaxWidth().hairlineBelow(c.rule).padding(vertical = 10.dp), verticalAlignment = Alignment.CenterVertically) {
                    GlyphIcon(deviceGlyph(device.kind), c.ink2, size = 20.dp)
                    Spacer(Modifier.width(12.dp))
                    Column(Modifier.weight(1f)) {
                        RText(dev.reins.android.ui.common.untrustedName(device.name), RType.row, c.ink, maxLines = 1)
                        RText(
                            "${device.platform} · signed in ${relativeTime(device.createdAt)} · last seen ${relativeTime(device.lastSeenAt)}",
                            RType.meta,
                            c.inkMeta,
                            maxLines = 2,
                        )
                    }
                }
                Row(Modifier.padding(top = 12.dp), verticalAlignment = Alignment.CenterVertically) {
                    RText("It can no longer open your vault or answer requests.", RType.sans(15f, lineHeight = 21f), c.ink2, Modifier.weight(1f))
                    HelpIcon(help, "signing a phone out") { help = it }
                }
                dev.reins.android.design.HelpText(
                    "It can't sign in again as it is. Someone with its screen lock could still read your recovery code on it " +
                        "(see I lost my phone in the help). Your recovery code or master password confirms it's you.",
                    help,
                    Modifier.padding(top = 4.dp),
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
                    dev.reins.android.design.SignalButton("Cancel", Modifier.weight(1f), style = ButtonStyle.Outline, onClick = onDismiss)
                    dev.reins.android.design.SignalButton(
                        "Sign out",
                        Modifier.weight(1f).testTag("confirmSignOut"),
                        style = ButtonStyle.DangerWash,
                        enabled = typed.isNotBlank() && !busy,
                    ) { onConfirm(typed) }
                }
            }
        }
    }
}

@Composable
private fun DeviceRow(device: DeviceView, busy: Boolean, onSignOut: () -> Unit) {
    val c = LocalColors.current
    // Next (Settings 02): the phone's tile, its name and when it was last seen, and Sign out in red.
    Row(
        Modifier.fillMaxWidth().testTag("device:${device.id}").padding(vertical = 10.dp),
        horizontalArrangement = Arrangement.spacedBy(12.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        androidx.compose.foundation.layout.Box(Modifier.size(32.dp).clip(androidx.compose.foundation.shape.RoundedCornerShape(9.dp)).background(c.paperSunken), contentAlignment = Alignment.Center) {
            GlyphIcon(deviceGlyph(device.kind), c.ink2, size = 18.dp)
        }
        Column(Modifier.weight(1f)) {
            RText(untrusted(device.name), RType.row, c.ink, maxLines = 1)
            RText("${device.platform} · ${relativeTime(device.lastSeenAt)}", RType.meta, c.inkMeta, maxLines = 2)
            if (device.approval) dev.reins.android.design.NextPill("Approves", c.success, Modifier.padding(top = 4.dp), glyph = Glyph.ShieldCheck)
        }
        RText(
            "Sign out",
            RType.sans(15f, FontWeight.SemiBold),
            c.danger,
            Modifier
                .clip(androidx.compose.foundation.shape.CircleShape)
                .pressable(enabled = !busy, shape = androidx.compose.foundation.shape.CircleShape, label = "Sign out", onClick = onSignOut)
                .padding(horizontal = 6.dp, vertical = 8.dp)
                .testTag("signOutDevice:${device.id}"),
            maxLines = 1,
        )
    }
}

@Composable
private fun ViewerRow(member: RosterMemberView, busy: Boolean, onRemove: () -> Unit) {
    val c = LocalColors.current
    Row(
        Modifier.fillMaxWidth().testTag("viewer:${member.deviceId}").padding(vertical = 10.dp),
        horizontalArrangement = Arrangement.spacedBy(12.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        androidx.compose.foundation.layout.Box(Modifier.size(32.dp).clip(androidx.compose.foundation.shape.RoundedCornerShape(9.dp)).background(c.paperSunken), contentAlignment = Alignment.Center) {
            GlyphIcon(Glyph.Phone, c.ink2, size = 18.dp)
        }
        RText(untrustedName(member.name), RType.row, c.ink, Modifier.weight(1f), maxLines = 1)
        RText(
            "Remove",
            RType.sans(15f, FontWeight.SemiBold),
            c.danger,
            Modifier
                .clip(androidx.compose.foundation.shape.CircleShape)
                .pressable(enabled = !busy, shape = androidx.compose.foundation.shape.CircleShape, label = "Remove", onClick = onRemove)
                .padding(horizontal = 6.dp, vertical = 8.dp)
                .testTag("removeViewer:${member.deviceId}"),
            maxLines = 1,
        )
    }
}

/** "Remove a second phone": which one, what removing does, and Remove held to confirm. */
@Composable
private fun RemoveViewerDialog(member: RosterMemberView, busy: Boolean, error: String?, onConfirm: () -> Unit, onDismiss: () -> Unit) {
    val c = LocalColors.current
    androidx.compose.ui.window.Dialog(
        onDismissRequest = onDismiss,
        properties = androidx.compose.ui.window.DialogProperties(usePlatformDefaultWidth = false),
    ) {
        dev.reins.android.feedback.DialogFeedback()
        androidx.compose.foundation.layout.Box(Modifier.fillMaxSize(), contentAlignment = Alignment.BottomCenter) {
            val shape = androidx.compose.foundation.shape.RoundedCornerShape(topStart = 28.dp, topEnd = 28.dp)
            Column(
                Modifier
                    .fillMaxWidth()
                    .clip(shape)
                    .background(c.paperRaised)
                    .windowInsetsPadding(androidx.compose.foundation.layout.WindowInsets.navigationBars)
                    .padding(start = 22.dp, end = 22.dp, top = 22.dp, bottom = 16.dp)
                    .testTag("removeViewerDialog"),
            ) {
                dev.reins.android.design.AskText("Remove a second phone", c.ink, target = "Remove", style = RType.serif(32f, 36f), minSize = 26.sp)
                Row(Modifier.padding(top = 12.dp).fillMaxWidth().hairlineBelow(c.rule).padding(vertical = 10.dp), verticalAlignment = Alignment.CenterVertically) {
                    GlyphIcon(Glyph.Phone, c.ink2, size = 20.dp)
                    Spacer(Modifier.width(12.dp))
                    RText(untrustedName(member.name), RType.row, c.ink, Modifier.weight(1f), maxLines = 1)
                }
                RText("It no longer sees this account.", RType.sans(15f, lineHeight = 21f), c.ink2, Modifier.padding(top = 12.dp))
                error?.let { Banner(it, Modifier.padding(top = 10.dp), BannerKind.Error, tag = "removeViewerError") }
                Row(Modifier.padding(top = 18.dp), horizontalArrangement = Arrangement.spacedBy(10.dp)) {
                    dev.reins.android.design.SignalButton("Cancel", Modifier.weight(1f), style = ButtonStyle.Outline, onClick = onDismiss)
                    dev.reins.android.design.HoldButton(
                        "Remove",
                        c.danger,
                        Modifier.weight(1f).testTag("confirmRemoveViewer"),
                        icon = null,
                        busy = busy,
                        onConfirm = onConfirm,
                    )
                }
            }
        }
    }
}
