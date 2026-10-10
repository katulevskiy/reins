package dev.reins.android.ui.autopilot

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.ExperimentalLayoutApi
import androidx.compose.foundation.layout.FlowRow
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
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import dev.reins.android.autopilot.AutopilotText
import dev.reins.android.design.Banner
import dev.reins.android.design.BannerKind
import dev.reins.android.design.ButtonStyle
import dev.reins.android.design.CapsuleButton
import dev.reins.android.design.ConfirmDialog
import dev.reins.android.design.Glyph
import dev.reins.android.design.Group
import dev.reins.android.design.Hairline
import dev.reins.android.design.LocalColors
import dev.reins.android.design.RText
import dev.reins.android.design.RType
import dev.reins.android.design.SelectChip
import dev.reins.android.design.rememberNowState
import dev.reins.android.ui.common.untrusted
import dev.reins.core.AutopilotMode

/**
 * The Autopilot part of an AI's page: its own mode (or the global one), a bypass for just this AI, and the profile
 * its answers teach.
 */
@OptIn(ExperimentalLayoutApi::class)
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
    val defaultProfile = ui.profiles.firstOrNull { it.id == s.defaultProfileId }
    val profileId = own?.profileId ?: s.defaultProfileId
    var bypassAsk by remember { mutableStateOf(false) }
    var lockdownAsk by remember { mutableStateOf(false) }

    Group(
        header = "Autopilot",
        footer = "Its own mode wins over the global one, except a global Lockdown. A bypass is not possible in an AI's first 10 minutes.",
    ) {
        Row(Modifier.fillMaxWidth().padding(16.dp).testTag("connectionMode"), verticalAlignment = Alignment.CenterVertically) {
            IconTile(modeGlyph(mode), modeTint(mode, c), size = 44.dp, filled = true)
            Spacer(Modifier.width(14.dp))
            Column(Modifier.weight(1f)) {
                RText(AutopilotText.name(mode), RType.sans(17f, FontWeight.SemiBold), c.text, maxLines = 1)
                val lockedDown = mode == AutopilotMode.LOCKDOWN && s.mode == AutopilotMode.LOCKDOWN && own?.baseMode != AutopilotMode.LOCKDOWN
                ModeLine(bypassUntil) { now ->
                    when {
                        lockedDown -> "Every AI is locked down"
                        bypassUntil != null -> "Bypass for this AI · " + AutopilotText.minutesLeft(bypassUntil, now)
                        chosen == null -> "Like every AI"
                        else -> "Its own mode"
                    }
                }
            }
            if (bypassUntil != null) {
                CapsuleButton("Stop", Modifier.testTag("stopConnectionBypass"), style = ButtonStyle.Destructive, compact = true, glyph = Glyph.Stop) {
                    viewModel.stopBypass(connectionId)
                }
            }
        }
        FlowRow(
            Modifier.padding(start = 16.dp, end = 16.dp, bottom = 16.dp),
            horizontalArrangement = Arrangement.spacedBy(8.dp),
            verticalArrangement = Arrangement.spacedBy(8.dp),
        ) {
            SelectChip("Like every AI", chosen == null, Modifier.testTag("connMode:follow")) {
                if (chosen != null) viewModel.setMode(null, connectionId = connectionId)
            }
            AutopilotText.modes.forEach { m ->
                SelectChip(AutopilotText.name(m), chosen == m, Modifier.testTag("connMode:${m.name}")) {
                    when {
                        m == AutopilotMode.BYPASS -> bypassAsk = true
                        m == AutopilotMode.LOCKDOWN && chosen != m -> lockdownAsk = true
                        m != chosen -> viewModel.setMode(m, connectionId = connectionId)
                    }
                }
            }
        }
        if (ui.profiles.isNotEmpty()) {
            Hairline()
            Column(Modifier.padding(16.dp)) {
                Caption("Learns into")
                FlowRow(
                    Modifier.padding(top = 10.dp),
                    horizontalArrangement = Arrangement.spacedBy(8.dp),
                    verticalArrangement = Arrangement.spacedBy(8.dp),
                ) {
                    ui.profiles.forEach { p ->
                        val title = "${AutopilotText.profileIcon(p)}  ${untrusted(p.name)}" + if (p.id == defaultProfile?.id) " (default)" else ""
                        SelectChip(title, p.id == profileId, Modifier.testTag("connProfile:${p.id}")) {
                            if (p.id != profileId) viewModel.assignProfile(connectionId, p.id.takeIf { it != s.defaultProfileId })
                        }
                    }
                }
            }
        }
        ui.error?.let { Banner(it, Modifier.padding(start = 16.dp, end = 16.dp, bottom = 16.dp), BannerKind.Error, tag = "connAutopilotError") }
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
 * The line under an AI's mode. A bypass that runs ([bypassUntil]) gives it a clock of its own, so the countdown
 * recomposes this line and not the section; [text] gets the time in unix seconds.
 */
@Composable
private fun ModeLine(bypassUntil: Long?, text: (nowSeconds: Long) -> String) {
    val c = LocalColors.current
    val now = rememberNowState(1_000, untilMillis = bypassUntil?.let { it * 1000 }).value / 1000
    RText(
        text(now),
        RType.sans(13f),
        if (bypassUntil != null) c.danger else c.secondary,
        Modifier.padding(top = 2.dp).testTag("connectionModeLine"),
        maxLines = 2,
    )
}
