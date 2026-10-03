package dev.reins.android.ui.settings

import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.ExperimentalLayoutApi
import androidx.compose.foundation.layout.FlowRow
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.runtime.Composable
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
import dev.reins.android.design.ButtonStyle
import dev.reins.android.design.CapsuleButton
import dev.reins.android.design.ConfirmDialog
import dev.reins.android.design.ConnectionAvatar
import dev.reins.android.design.EmptyState
import dev.reins.android.design.Glyph
import dev.reins.android.design.Group
import dev.reins.android.design.Hairline
import dev.reins.android.design.ListRow
import dev.reins.android.design.LocalColors
import dev.reins.android.design.Providers
import dev.reins.android.design.RText
import dev.reins.android.design.RType
import dev.reins.android.design.Screen
import dev.reins.android.design.pressable
import dev.reins.android.state.AppState
import dev.reins.android.ui.common.formatFull
import dev.reins.android.ui.common.relativeTime
import dev.reins.android.ui.common.untrusted

/** One AI connection: its icon (a provider's or one generated from the name), its Autopilot mode and profile, history, disconnect. */
@OptIn(ExperimentalLayoutApi::class)
@Composable
fun ConnectionDetailScreen(
    connectionId: String,
    state: AppState,
    viewModel: SettingsViewModel,
    autopilot: dev.reins.android.ui.autopilot.AutopilotViewModel? = null,
    onBack: () -> Unit,
) {
    val c = LocalColors.current
    val connections by state.connections.collectAsStateWithLifecycle()
    val grants by state.grants.collectAsStateWithLifecycle()
    val connection = connections.firstOrNull { it.id == connectionId }
    var confirming by remember { mutableStateOf(false) }

    Screen(title = connection?.let { untrusted(it.label) } ?: "Connection", onBack = onBack) {
        if (connection == null) {
            EmptyState(Glyph.Link, "Not connected", "This AI is no longer connected.", tag = "connectionGone")
            return@Screen
        }
        Column(Modifier.fillMaxWidth().padding(vertical = 8.dp), horizontalAlignment = Alignment.CenterHorizontally) {
            ConnectionAvatar(untrusted(connection.label), connection.icon, size = 84.dp)
            RText(untrusted(connection.label), RType.sans(24f, FontWeight.SemiBold), c.text, Modifier.padding(top = 12.dp))
            RText(untrusted(connection.clientHost), RType.mono(13f), c.secondary, ltr = true)
        }

        autopilot?.let { dev.reins.android.ui.autopilot.ConnectionAutopilotSection(connection.id, connection.label, it) }

        RText("ICON", RType.sans(12.5f, FontWeight.Medium), c.secondary, Modifier.padding(start = 32.dp, top = 16.dp, bottom = 8.dp))
        FlowRow(
            Modifier.padding(horizontal = 16.dp),
            horizontalArrangement = Arrangement.spacedBy(12.dp),
            verticalArrangement = Arrangement.spacedBy(12.dp),
        ) {
            IconChoice("auto", "Auto", selected = connection.icon == null, avatarPick = null, label = connection.label) {
                viewModel.pickIcon(connection.id, null)
            }
            IconChoice("blob", "Blobatar", selected = connection.icon == "blob", avatarPick = "blob", label = connection.label) {
                viewModel.pickIcon(connection.id, "blob")
            }
            Providers.all.forEach { provider ->
                IconChoice(provider.key, provider.name, selected = connection.icon == provider.key, avatarPick = provider.key, label = provider.name) {
                    viewModel.pickIcon(connection.id, provider.key)
                }
            }
        }
        RText(
            "Auto picks a known AI from the name, or draws a blobatar for it.",
            RType.sans(12.5f),
            c.tertiary,
            Modifier.padding(start = 32.dp, top = 8.dp, end = 32.dp),
        )

        Group(header = "Activity") {
            ListRow("Connected", subtitle = formatFull(connection.createdAt))
            Hairline()
            ListRow("Last used", subtitle = connection.lastUsedAt?.let { "${relativeTime(it)} · ${formatFull(it)}" } ?: "Never")
            Hairline()
            val active = grants.count { it.connectionId == connection.id && it.active }
            ListRow("Active grants", subtitle = if (active == 0) "None" else "$active")
        }

        Column(Modifier.padding(16.dp)) {
            CapsuleButton(
                "Disconnect",
                Modifier.fillMaxWidth().testTag("disconnect"),
                style = ButtonStyle.Destructive,
                glyph = Glyph.Trash,
            ) { confirming = true }
        }
        Spacer(Modifier.height(24.dp))
    }

    if (confirming && connection != null) {
        ConfirmDialog(
            title = "Disconnect ${untrusted(connection.label)}?",
            text = "It loses access immediately, and its saved grants stop working.",
            confirmLabel = "Disconnect",
            onConfirm = {
                confirming = false
                viewModel.disconnect(connection.id, onBack)
            },
            onDismiss = { confirming = false },
        )
    }
}

@Composable
private fun IconChoice(tag: String, name: String, selected: Boolean, avatarPick: String?, label: String, onClick: () -> Unit) {
    val c = LocalColors.current
    Column(Modifier.pressable(shape = CircleShape, onClick = onClick).testTag("icon:$tag"), horizontalAlignment = Alignment.CenterHorizontally) {
        Box(
            Modifier
                .border(if (selected) 2.5.dp else 0.dp, if (selected) c.accent else androidx.compose.ui.graphics.Color.Transparent, CircleShape)
                .padding(3.dp)
                .background(androidx.compose.ui.graphics.Color.Transparent, CircleShape),
        ) { ConnectionAvatar(label, avatarPick, size = 52.dp) }
        RText(name, RType.sans(12f, if (selected) FontWeight.SemiBold else FontWeight.Normal), if (selected) c.accent else c.secondary, Modifier.padding(top = 4.dp), maxLines = 1)
    }
}
