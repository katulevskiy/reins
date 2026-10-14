package dev.reins.android.ui.settings

import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.ExperimentalLayoutApi
import androidx.compose.foundation.layout.FlowRow
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.rounded.Delete
import androidx.compose.material.icons.rounded.History
import androidx.compose.material.icons.rounded.Key
import androidx.compose.material.icons.rounded.Laptop
import androidx.compose.runtime.Composable
import androidx.compose.runtime.setValue
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.semantics.heading
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.dp
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import dev.reins.android.design.ConnectionAvatar
import dev.reins.android.design.EmptyState
import dev.reins.android.design.Glyph
import dev.reins.android.design.LocalColors
import dev.reins.android.design.Providers
import dev.reins.android.design.RText
import dev.reins.android.design.RType
import dev.reins.android.design.pressable
import dev.reins.android.state.AppState
import dev.reins.android.ui.autopilot.AutopilotViewModel
import dev.reins.android.ui.autopilot.ConnectionAutopilotSection
import dev.reins.android.ui.autopilot.LearnsIntoRow
import dev.reins.android.ui.autopilot.ProfilePicker
import dev.reins.android.ui.common.relativeTime
import dev.reins.android.ui.common.formatDate
import dev.reins.android.ui.common.untrusted
import dev.reins.core.ConnectionView

/**
 * One AI app or computer: its icon and name, its Autopilot mode (like every AI, its own, a bypass or a lockdown), the
 * profile it teaches, its grants, when it connected, its icon, and disconnecting it.
 */
@Composable
fun ConnectionDetailScreen(
    connectionId: String,
    state: AppState,
    viewModel: SettingsViewModel,
    autopilot: AutopilotViewModel? = null,
    onBack: () -> Unit,
) {
    val c = LocalColors.current
    val connections by state.connections.collectAsStateWithLifecycle()
    val grants by state.grants.collectAsStateWithLifecycle()
    val connection = connections.firstOrNull { it.id == connectionId }
    var picking by remember { mutableStateOf<Picker?>(null) }

    Box(Modifier.fillMaxSize()) {
        SettingsPage(title = null, onBack = onBack) {
            if (connection == null) {
                EmptyState(Glyph.Link, "Not connected", "This AI is no longer connected.", tag = "connectionGone")
                return@SettingsPage
            }
            Column(Modifier.fillMaxWidth().padding(top = 4.dp, bottom = 6.dp, start = 24.dp, end = 24.dp), horizontalAlignment = Alignment.CenterHorizontally) {
                if (connection.keyFingerprint != null && connection.icon == null) {
                    IconPlate(Icons.Rounded.Laptop, c.pair, size = 76.dp)
                } else {
                    ConnectionAvatar(untrusted(connection.label), connection.icon, size = 76.dp)
                }
                RText(
                    untrusted(connection.label),
                    RType.sans(24f, FontWeight.SemiBold),
                    c.text,
                    Modifier.padding(top = 12.dp).semantics { heading() }.testTag("connectionName"),
                    maxLines = 2,
                    align = TextAlign.Center,
                )
                RText(
                    connectionHeadline(connection),
                    RType.mono(13f),
                    c.tertiary,
                    Modifier.padding(top = 2.dp).testTag("connectionHeadline"),
                    maxLines = 1,
                    ltr = true,
                )
            }

            autopilot?.let { ConnectionAutopilotSection(connection.id, connection.label, it) }

            SettingsGroup(header = null, gap = 12.dp) {
                autopilot?.let { LearnsIntoRow(connection.id, it) { picking = Picker.Profile } }
                val active = grants.count { it.connectionId == connection.id && it.active }
                SettingsRow(
                    "Active grants",
                    Modifier.testTag("activeGrants"),
                    icon = Icons.Rounded.Key,
                    tint = c.send,
                    value = if (active == 0) "None" else "$active",
                    chevron = false,
                )
                SettingsRow(
                    "Connected",
                    Modifier.testTag("connectedOn"),
                    icon = Icons.Rounded.History,
                    value = formatDate(connection.createdAt),
                    chevron = false,
                )
            }

            SettingsGroup(header = null, gap = 12.dp) {
                SettingsRow(
                    "Icon",
                    Modifier.testTag("connectionIcon"),
                    leading = {
                        Box(Modifier.size(40.dp).background(c.controlFill, CircleShape), contentAlignment = Alignment.Center) {
                            ConnectionAvatar(untrusted(connection.label), connection.icon, size = 28.dp)
                        }
                    },
                    value = iconName(connection.icon),
                ) { picking = Picker.Icon }
            }

            dev.reins.android.design.HoldButton(
                "Hold to disconnect",
                c.danger,
                Modifier.padding(start = 16.dp, end = 16.dp, top = 22.dp).fillMaxWidth().testTag("disconnect"),
                icon = Icons.Rounded.Delete,
            ) { viewModel.disconnect(connection.id, onBack) }
            Spacer(Modifier.height(32.dp))
        }

        val shown = picking
        BottomSheet(shown == Picker.Icon && connection != null, "Icon", onClose = { picking = null }, tag = "iconSheet") {
            if (connection != null) IconPicker(connection) { icon ->
                viewModel.pickIcon(connection.id, icon)
                picking = null
            }
        }
        BottomSheet(shown == Picker.Profile && autopilot != null, "Learns into", onClose = { picking = null }, tag = "profileSheet") {
            if (connection != null && autopilot != null) ProfilePicker(connection.id, autopilot) { picking = null }
        }
    }
}

private enum class Picker { Icon, Profile }

/** "claude.ai · used 5 min ago"; for a computer its key, compared when it paired: "Key 4821 9930 · used just now". */
internal fun connectionHeadline(connection: ConnectionView): String {
    val used = connection.lastUsedAt?.let { "used ${relativeTime(it)}" } ?: "never used"
    val where = connection.keyFingerprint?.let { "Key $it" } ?: untrusted(connection.clientHost)
    return "$where · $used"
}

/** The name of an icon choice: Auto (from the name), Blobatar, or the provider's. */
internal fun iconName(pick: String?): String = when (pick) {
    null -> "Auto"
    "blob" -> "Blobatar"
    else -> Providers.byKey(pick)?.name ?: "Auto"
}

@OptIn(ExperimentalLayoutApi::class)
@Composable
private fun IconPicker(connection: ConnectionView, onPick: (String?) -> Unit) {
    FlowRow(
        Modifier.padding(horizontal = 20.dp, vertical = 8.dp).fillMaxWidth(),
        horizontalArrangement = Arrangement.spacedBy(12.dp),
        verticalArrangement = Arrangement.spacedBy(14.dp),
    ) {
        IconChoice("auto", "Auto", selected = connection.icon == null, avatarPick = null, label = connection.label) { onPick(null) }
        IconChoice("blob", "Blobatar", selected = connection.icon == "blob", avatarPick = "blob", label = connection.label) { onPick("blob") }
        Providers.all.forEach { provider ->
            IconChoice(provider.key, provider.name, selected = connection.icon == provider.key, avatarPick = provider.key, label = provider.name) {
                onPick(provider.key)
            }
        }
    }
}

@Composable
private fun IconChoice(tag: String, name: String, selected: Boolean, avatarPick: String?, label: String, onClick: () -> Unit) {
    val c = LocalColors.current
    Column(Modifier.pressable(shape = CircleShape, onClick = onClick).testTag("icon:$tag"), horizontalAlignment = Alignment.CenterHorizontally) {
        Box(
            Modifier
                .border(if (selected) 2.5.dp else 0.dp, if (selected) c.accent else Color.Transparent, CircleShape)
                .padding(3.dp),
        ) { ConnectionAvatar(untrusted(label), avatarPick, size = 52.dp) }
        RText(name, RType.sans(12f, if (selected) FontWeight.SemiBold else FontWeight.Normal), if (selected) c.accent else c.secondary, Modifier.padding(top = 4.dp), maxLines = 1)
    }
}
