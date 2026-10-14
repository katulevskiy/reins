package dev.reins.android.ui.autopilot

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.rounded.Bolt
import androidx.compose.material.icons.rounded.Check
import androidx.compose.material.icons.rounded.Lock
import androidx.compose.material.icons.rounded.LockOpen
import androidx.compose.material.icons.rounded.SmartToy
import androidx.compose.material.icons.rounded.Stop
import androidx.compose.material3.Icon
import androidx.compose.runtime.Composable
import androidx.compose.runtime.setValue
import androidx.compose.runtime.getValue
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.unit.dp
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import dev.reins.android.autopilot.AutopilotText
import dev.reins.android.design.Banner
import dev.reins.android.design.BannerKind
import dev.reins.android.design.ConfirmDialog
import dev.reins.android.design.LocalColors
import dev.reins.android.design.RText
import dev.reins.android.design.RType
import dev.reins.android.design.rememberNowState
import dev.reins.android.ui.common.untrusted
import dev.reins.android.ui.settings.ConnectedButtons
import dev.reins.android.ui.settings.PillButton
import dev.reins.android.ui.settings.SettingsGroup
import dev.reins.android.ui.settings.SettingsRow
import dev.reins.android.ui.settings.modeIcon
import dev.reins.core.AutopilotMode

/** Its own mode wins over the global one: what the (?) beside "Autopilot" on an AI's page explains. */
const val CONNECTION_AUTOPILOT_HELP =
    "Its own mode wins over the global one, except a global Lockdown. A bypass is not possible in an AI's first 10 minutes."

/**
 * The Autopilot part of an AI's page: like every AI, or its own mode (Manual, Auto) in a connected button group, then
 * a bypass for just this AI and a lockdown for just this AI, each after its own confirmation.
 */
@Composable
fun ConnectionAutopilotSection(connectionId: String, label: String, viewModel: AutopilotViewModel) {
    val c = LocalColors.current
    val settings by viewModel.settings.collectAsStateWithLifecycle()
    val ui by viewModel.ui.collectAsStateWithLifecycle()
    LaunchedEffect(viewModel) { viewModel.showingProfiles() }
    val s = settings ?: return
    val own = s.connections.firstOrNull { it.connectionId == connectionId }
    val mode = own?.mode ?: s.mode
    val bypassUntil = own?.bypassUntil
    val chosen: AutopilotMode? = if (bypassUntil != null) AutopilotMode.BYPASS else own?.baseMode
    val lockedDown = chosen == AutopilotMode.LOCKDOWN
    val globalLockdown = mode == AutopilotMode.LOCKDOWN && s.mode == AutopilotMode.LOCKDOWN && own?.baseMode != AutopilotMode.LOCKDOWN
    var bypassAsk by remember { mutableStateOf(false) }
    var lockdownAsk by remember { mutableStateOf(false) }
    var help by rememberSaveable { mutableStateOf(false) }

    // The choices of the group: like every AI (null), then the modes that are not one of the two buttons below.
    val choices: List<AutopilotMode?> = listOf<AutopilotMode?>(null) +
        AutopilotText.modes.filter { it != AutopilotMode.BYPASS && it != AutopilotMode.LOCKDOWN }

    SettingsGroup(header = "Autopilot", help = CONNECTION_AUTOPILOT_HELP, helpOpen = help, onHelp = { help = it }) {
        Column(Modifier.padding(12.dp).testTag("connectionMode")) {
            ConnectedButtons(
                options = choices,
                // A bypass or a lockdown of its own is one of the buttons below: then none of these is chosen.
                selected = { it == chosen },
                label = { it?.let(AutopilotText::name) ?: "Like all" },
                tag = { if (it == null) "connMode:follow" else "connMode:${it.name}" },
                icon = { m -> m?.let { modeIcon(it) to if (it == AutopilotMode.MANUAL) modeTint(it, c) else c.secondary } },
            ) { m -> viewModel.setMode(m, connectionId = connectionId) }
            Row(Modifier.padding(top = 12.dp), horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                if (bypassUntil != null) {
                    BypassCountdown(bypassUntil, Modifier.weight(1f)) { viewModel.stopBypass(connectionId) }
                } else {
                    PillButton(
                        "Bypass",
                        Modifier.weight(1f).testTag("connMode:BYPASS"),
                        icon = Icons.Rounded.Bolt,
                        background = c.danger.copy(alpha = 0.12f),
                        foreground = c.danger,
                    ) { bypassAsk = true }
                }
                if (lockedDown) {
                    PillButton(
                        "Unlock",
                        Modifier.weight(1f).testTag("connMode:LOCKDOWN"),
                        icon = Icons.Rounded.LockOpen,
                        background = c.warning.copy(alpha = 0.14f),
                        foreground = c.warning,
                    ) { viewModel.setMode(null, connectionId = connectionId) }
                } else {
                    PillButton(
                        "Lock down",
                        Modifier.weight(1f).testTag("connMode:LOCKDOWN"),
                        icon = Icons.Rounded.Lock,
                        background = if (c.dark) Color.White.copy(alpha = 0.06f) else c.controlFill,
                        foreground = c.text,
                    ) { lockdownAsk = true }
                }
            }
            if (globalLockdown) {
                RText(
                    "Every AI is locked down",
                    RType.sans(13f),
                    c.warning,
                    Modifier.padding(start = 4.dp, top = 10.dp).testTag("connectionModeLine"),
                    maxLines = 2,
                )
            }
            ui.error?.let { Banner(it, Modifier.padding(top = 12.dp), BannerKind.Error, tag = "connAutopilotError") }
        }
    }

    if (bypassAsk) {
        BypassDialog(
            who = label,
            onConfirm = { minutes ->
                bypassAsk = false
                viewModel.setMode(AutopilotMode.BYPASS, minutes, connectionId)
            },
            onDismiss = { bypassAsk = false },
        )
    }
    if (lockdownAsk) {
        ConfirmDialog(
            title = "Lock down ${untrusted(label)}?",
            text = "Only this AI. Waiting requests too.",
            confirmLabel = "Lock down",
            onConfirm = {
                lockdownAsk = false
                viewModel.setMode(AutopilotMode.LOCKDOWN, connectionId = connectionId)
            },
            onDismiss = { lockdownAsk = false },
        )
    }
}

/**
 * A running bypass in place of its button: "Stop · 12 min left". It has a clock of its own, so the countdown
 * recomposes this button and not the section.
 */
@Composable
private fun BypassCountdown(until: Long, modifier: Modifier, onStop: () -> Unit) {
    val c = LocalColors.current
    val now = rememberNowState(1_000, untilMillis = until * 1000).value / 1000
    PillButton(
        "Stop · " + AutopilotText.minutesLeft(until, now),
        modifier.testTag("stopConnectionBypass"),
        icon = Icons.Rounded.Stop,
        background = c.danger.copy(alpha = 0.16f),
        foreground = c.danger,
        textTag = "connectionModeLine",
        onClick = onStop,
    )
}

/** "Learns into": the profile this AI's answers teach, opening the picker. Nothing until the profiles are read. */
@Composable
fun LearnsIntoRow(connectionId: String, viewModel: AutopilotViewModel, onClick: () -> Unit) {
    val c = LocalColors.current
    val settings by viewModel.settings.collectAsStateWithLifecycle()
    val ui by viewModel.ui.collectAsStateWithLifecycle()
    val s = settings ?: return
    if (ui.profiles.isEmpty()) return
    val profileId = s.connections.firstOrNull { it.connectionId == connectionId }?.profileId ?: s.defaultProfileId
    val profile = ui.profiles.firstOrNull { it.id == profileId }
    SettingsRow(
        "Learns into",
        Modifier.testTag("learnsInto"),
        icon = Icons.Rounded.SmartToy,
        tint = c.accent,
        value = profile?.let { untrusted(it.name) },
        onClick = onClick,
    )
}

/** The profiles to pick from for an AI, the one it teaches ticked. Picking one closes the picker. */
@Composable
fun ProfilePicker(connectionId: String, viewModel: AutopilotViewModel, onPicked: () -> Unit) {
    val c = LocalColors.current
    val settings by viewModel.settings.collectAsStateWithLifecycle()
    val ui by viewModel.ui.collectAsStateWithLifecycle()
    val s = settings ?: return
    val profileId = s.connections.firstOrNull { it.connectionId == connectionId }?.profileId ?: s.defaultProfileId
    Column(Modifier.padding(horizontal = 16.dp).fillMaxWidth()) {
        ui.profiles.forEach { p ->
            SettingsRow(
                untrusted(p.name),
                Modifier.testTag("connProfile:${p.id}"),
                subtitle = if (p.id == s.defaultProfileId) "Default" else null,
                chevron = false,
                valueContent = if (p.id == profileId) {
                    { Icon(Icons.Rounded.Check, null, Modifier.size(22.dp), tint = c.accent) }
                } else null,
            ) {
                if (p.id != profileId) viewModel.assignProfile(connectionId, p.id.takeIf { it != s.defaultProfileId })
                onPicked()
            }
        }
    }
}
