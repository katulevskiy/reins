package dev.reins.android.ui.connections

import dev.reins.android.ui.settings.*

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
import androidx.compose.material.icons.rounded.Cancel
import androidx.compose.material.icons.rounded.Delete
import androidx.compose.material.icons.rounded.Edit
import androidx.compose.material.icons.rounded.History
import androidx.compose.material.icons.rounded.Key
import androidx.compose.material.icons.rounded.Schedule
import androidx.compose.material.icons.rounded.Shuffle
import androidx.compose.material3.Icon
import androidx.compose.runtime.Composable
import androidx.compose.runtime.setValue
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.heading
import androidx.compose.ui.semantics.selected
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.dp
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import dev.reins.android.design.BLOB_ICON
import dev.reins.android.design.BlobAvatar
import dev.reins.android.design.ButtonSize
import dev.reins.android.design.ButtonStyle
import dev.reins.android.design.ConnectionAvatar
import dev.reins.android.design.EmptyState
import dev.reins.android.design.Glyph
import dev.reins.android.design.LabelText
import dev.reins.android.design.LocalColors
import dev.reins.android.design.Platforms
import dev.reins.android.design.Providers
import dev.reins.android.design.RText
import dev.reins.android.design.RType
import dev.reins.android.design.SignalButton
import dev.reins.android.design.autoIcon
import dev.reins.android.design.blobatar.Blobatar
import dev.reins.android.design.iconName
import dev.reins.android.design.pressable
import dev.reins.android.design.shownIcon
import dev.reins.android.state.AppState
import dev.reins.android.ui.autopilot.AutopilotViewModel
import dev.reins.android.platform.Authenticator
import dev.reins.android.ui.autopilot.BypassSheets
import dev.reins.android.ui.autopilot.OvernightSheets
import dev.reins.android.ui.autopilot.ConnectionAutopilotSection
import dev.reins.android.ui.autopilot.LearnsIntoRow
import dev.reins.android.ui.autopilot.ProfilePicker
import dev.reins.android.ui.common.relativeTime
import dev.reins.android.ui.common.formatDate
import dev.reins.android.ui.common.untrusted
import dev.reins.android.ui.common.untrustedName
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
    /** The phone owner's check, before a bypass of this AI starts. */
    authenticator: Authenticator? = null,
) {
    val c = LocalColors.current
    val connections by state.connections.collectAsStateWithLifecycle()
    val grants by state.grants.collectAsStateWithLifecycle()
    val connection = connections.firstOrNull { it.id == connectionId }
    var picking by remember { mutableStateOf<Picker?>(null) }
    var bypassOpen by remember { mutableStateOf(false) }
    var overnightOpen by remember { mutableStateOf(false) }

    Box(Modifier.fillMaxSize()) {
        SettingsPage(title = null, onBack = onBack) {
            if (connection == null) {
                EmptyState(Glyph.Link, "Not connected", "This AI is no longer connected.", tag = "connectionGone")
                return@SettingsPage
            }
            Column(Modifier.fillMaxWidth().padding(top = 4.dp, bottom = 6.dp, start = 24.dp, end = 24.dp), horizontalAlignment = Alignment.CenterHorizontally) {
                ConnectionAvatar(connection, Modifier.testTag("connectionAvatar:${shownIcon(connection)}"), size = 76.dp)
                RText(
                    untrusted(connection.label),
                    RType.serif(34f, 38f),
                    c.ink,
                    Modifier.padding(top = 12.dp).semantics { heading() }.testTag("connectionName"),
                    maxLines = 2,
                    align = TextAlign.Center,
                )
                calledItself(connection)?.let {
                    RText(it, RType.meta, c.inkMeta, Modifier.padding(top = 2.dp).testTag("connectionCalledItself"), maxLines = 1)
                }
                RText(
                    connectionHeadline(connection),
                    RType.mono(13f),
                    c.tertiary,
                    Modifier.padding(top = 2.dp).testTag("connectionHeadline"),
                    maxLines = 1,
                    ltr = true,
                )
            }

            autopilot?.let { ConnectionAutopilotSection(connection.id, it, onBypass = { bypassOpen = true }, onOvernight = { overnightOpen = true }, authenticator = authenticator) }

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
                SettingsRow(
                    "Last used",
                    Modifier.testTag("lastUsed"),
                    icon = Icons.Rounded.Schedule,
                    value = lastUsed(connection),
                    chevron = false,
                )
            }

            SettingsGroup(header = null, gap = 12.dp) {
                SettingsRow(
                    "Name",
                    Modifier.testTag("connectionRename"),
                    icon = Icons.Rounded.Edit,
                    value = untrustedName(connection.label, 28),
                ) { picking = Picker.Name }
                SettingsRow(
                    "Icon",
                    Modifier.testTag("connectionIcon"),
                    leading = {
                        Box(Modifier.size(40.dp).background(c.controlFill, CircleShape), contentAlignment = Alignment.Center) {
                            ConnectionAvatar(connection, size = 28.dp)
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
            if (connection != null) {
                IconPicker(
                    connection,
                    onPick = { icon ->
                        viewModel.pickIcon(connection.id, icon)
                        picking = null
                    },
                    onShape = { viewModel.editBlobatar(connection.id, shownIcon(connection), shape = it) },
                    onColour = { viewModel.editBlobatar(connection.id, shownIcon(connection), colour = it) },
                    onShuffle = { viewModel.editBlobatar(connection.id, shownIcon(connection)) },
                )
            }
        }
        BottomSheet(shown == Picker.Name && connection != null, "Name", onClose = { picking = null }, tag = "renameSheet") {
            if (connection != null) RenameField(connection) { name -> viewModel.rename(connection.id, name) { picking = null } }
        }
        BottomSheet(shown == Picker.Profile && autopilot != null, "Learns into", onClose = { picking = null }, tag = "profileSheet") {
            if (connection != null && autopilot != null) ProfilePicker(connection.id, autopilot) { picking = null }
        }
        // Without the owner's check there is no bypass or Overnight here at all.
        if (autopilot != null && connection != null && authenticator != null) {
            BypassSheets(
                bypassOpen,
                who = connection.label,
                onStart = { minutes -> autopilot.startBypass(minutes, connection.id, connection.label, authenticator) },
                onClose = { bypassOpen = false },
            )
            val apSettings by autopilot.settings.collectAsStateWithLifecycle()
            OvernightSheets(
                overnightOpen,
                who = connection.label,
                risky = apSettings?.overnightWhenRisky ?: dev.reins.core.OvernightRisky.WAIT,
                ratesRisk = apSettings?.model?.risk == true,
                onRisky = { autopilot.setOvernightWhenRisky(it, authenticator) },
                onStart = { minutes -> autopilot.startOvernight(minutes, connection.id, connection.label, authenticator) },
                onClose = { overnightOpen = false },
            )
        }
    }
}

private enum class Picker { Icon, Profile, Name }

/**
 * The quiet line under a renamed connection's name, or a numbered one's ("Claude 2"): what it calls itself, quoted
 * ("Calls itself “Claude”"), so that a name never hides what it is. Null while it goes by its own name.
 */
internal fun calledItself(connection: ConnectionView): String? {
    if (connection.name == null && connection.number == null) return null
    val own = untrustedName(connection.clientName.ifEmpty { connection.ownLabel }, 48)
    return if (own.isEmpty()) null else "Calls itself “$own”"
}

/** "5 min ago", "Never": the Last used row. */
internal fun lastUsed(connection: ConnectionView): String = connection.lastUsedAt?.let { relativeTime(it) } ?: "Never"

/**
 * The name field: the user's name for it, its own as the placeholder; the clear button (or an empty field) gives it
 * its own name back. What it calls itself stays under the field.
 */
@Composable
private fun RenameField(connection: ConnectionView, onSave: (String?) -> Unit) {
    val c = LocalColors.current
    var text by remember(connection.id) { mutableStateOf(connection.name.orEmpty()) }
    val focus = remember { androidx.compose.ui.focus.FocusRequester() }
    androidx.compose.runtime.LaunchedEffect(connection.id) { runCatching { focus.requestFocus() } }
    val own = untrustedName(connection.ownLabel.ifEmpty { connection.label }, 64)
    Column(Modifier.fillMaxWidth().padding(horizontal = 4.dp)) {
        androidx.compose.foundation.layout.Row(verticalAlignment = Alignment.CenterVertically) {
            dev.reins.android.design.RTextField(
                text,
                { text = it.take(64) },
                own,
                Modifier.weight(1f),
                tag = "renameField",
                focusRequester = focus,
            )
            if (text.isNotEmpty()) {
                Box(
                    Modifier.padding(start = 8.dp).size(40.dp).clip(CircleShape)
                        .pressable(shape = CircleShape, label = "Use its own name", onClick = { text = "" })
                        .testTag("renameClear"),
                    contentAlignment = Alignment.Center,
                ) { Icon(Icons.Rounded.Cancel, "Use its own name", tint = c.tertiary) }
            }
        }
        RText(
            "Calls itself “${untrustedName(connection.clientName.ifEmpty { own }, 48)}”",
            RType.meta,
            c.inkMeta,
            Modifier.padding(top = 10.dp, start = 4.dp).testTag("renameItself"),
            maxLines = 1,
        )
        SignalButton(
            "Save",
            onClick = { onSave(text.trim().ifEmpty { null }) },
            modifier = Modifier.padding(top = 18.dp).fillMaxWidth().testTag("renameSave"),
            style = ButtonStyle.Inverse,
            size = ButtonSize.Large,
            glyph = Glyph.Check,
        )
    }
}

/** "claude.ai · used 5 min ago"; for a computer its key, compared when it paired: "Key 4821 9930 · used just now". */
internal fun connectionHeadline(connection: ConnectionView): String {
    val used = connection.lastUsedAt?.let { "used ${relativeTime(it)}" } ?: "never used"
    val where = connection.keyFingerprint?.let { "Key $it" } ?: untrusted(connection.clientHost)
    return "$where · $used"
}

/**
 * The Icon sheet: the icons to pick (Auto, drawn as what it resolves to; the blobatar; for a computer the platforms'
 * logos; the providers'), then the blobatar's shape and colour and a shuffle.
 */
@OptIn(ExperimentalLayoutApi::class)
@Composable
private fun IconPicker(
    connection: ConnectionView,
    onPick: (String?) -> Unit,
    onShape: (Int) -> Unit,
    onColour: (Int) -> Unit,
    onShuffle: () -> Unit,
) {
    FlowRow(
        Modifier.padding(horizontal = 20.dp, vertical = 8.dp).fillMaxWidth(),
        horizontalArrangement = Arrangement.spacedBy(12.dp),
        verticalArrangement = Arrangement.spacedBy(14.dp),
    ) {
        IconChoice("auto", "Auto", selected = connection.icon == null, connection, autoIcon(connection)) { onPick(null) }
        IconChoice(BLOB_ICON, "Blobatar", selected = connection.icon == BLOB_ICON, connection, BLOB_ICON) { onPick(BLOB_ICON) }
        if (connection.keyFingerprint != null) {
            Platforms.all.forEach { platform ->
                IconChoice(platform.key, platform.name, selected = connection.icon == platform.key, connection, platform.key) { onPick(platform.key) }
            }
        }
        Providers.all.forEach { provider ->
            IconChoice(provider.key, provider.name, selected = connection.icon == provider.key, connection, provider.key) { onPick(provider.key) }
        }
    }
    BlobatarEditor(connection, onShape, onColour, onShuffle)
}

@Composable
private fun IconChoice(tag: String, name: String, selected: Boolean, connection: ConnectionView, shown: String, onClick: () -> Unit) {
    val c = LocalColors.current
    Column(Modifier.pressable(shape = CircleShape, onClick = onClick).testTag("icon:$tag"), horizontalAlignment = Alignment.CenterHorizontally) {
        Box(
            Modifier
                .border(if (selected) 2.5.dp else 0.dp, if (selected) c.accent else Color.Transparent, CircleShape)
                .padding(3.dp),
        ) { ConnectionAvatar(connection, size = 52.dp, shown = shown) }
        RText(name, RType.sans(12f, if (selected) FontWeight.SemiBold else FontWeight.Normal), if (selected) c.accent else c.secondary, Modifier.padding(top = 4.dp), maxLines = 1)
    }
}

/**
 * The blobatar's own choices: its shapes (each drawn in its colour), its colours, and Shuffle (a new figure, shape and
 * colour). The one it keeps is ringed.
 */
@OptIn(ExperimentalLayoutApi::class)
@Composable
private fun BlobatarEditor(connection: ConnectionView, onShape: (Int) -> Unit, onColour: (Int) -> Unit, onShuffle: () -> Unit) {
    val c = LocalColors.current
    val kept = connection.blobatar
    val seed = kept?.seed ?: untrusted(connection.label)
    val colour = kept?.colour?.toInt()
    Column(Modifier.padding(horizontal = 20.dp).padding(top = 18.dp).fillMaxWidth().testTag("blobatarEditor")) {
        LabelText("Blobatar", c.inkMeta, Modifier.padding(bottom = 10.dp))
        FlowRow(
            Modifier.fillMaxWidth(),
            horizontalArrangement = Arrangement.SpaceBetween,
            verticalArrangement = Arrangement.spacedBy(8.dp),
            maxItemsInEachRow = 5,
        ) {
            Blobatar.SHAPES.indices.forEach { i ->
                Ringed("shape:$i", "Shape ${i + 1}", selected = kept?.shape?.toInt() == i, onClick = { onShape(i) }) {
                    BlobAvatar(seed, size = 46.dp, shape = i, colour = colour)
                }
            }
        }
        FlowRow(
            Modifier.fillMaxWidth().padding(top = 14.dp),
            horizontalArrangement = Arrangement.SpaceBetween,
            verticalArrangement = Arrangement.spacedBy(8.dp),
            maxItemsInEachRow = 6,
        ) {
            Blobatar.COLOURS.forEachIndexed { i, swatch ->
                Ringed("colour:$i", swatch.name, selected = colour == i, onClick = { onColour(i) }) {
                    Box(Modifier.size(34.dp).clip(CircleShape).background(headColor(swatch)))
                }
            }
        }
        val (_, fg) = ButtonStyle.Soft.colors(c)
        SignalButton(
            "Shuffle",
            Modifier.padding(top = 16.dp).align(Alignment.CenterHorizontally).testTag("shuffleBlobatar"),
            style = ButtonStyle.Soft,
            size = ButtonSize.Small,
            leading = { Icon(Icons.Rounded.Shuffle, null, Modifier.size(18.dp), tint = fg) },
            onClick = onShuffle,
        )
    }
}

/** One choice of the blobatar editor, ringed when it is the one kept; [label] is what a screen reader says. */
@Composable
private fun Ringed(tag: String, label: String, selected: Boolean, onClick: () -> Unit, content: @Composable () -> Unit) {
    val c = LocalColors.current
    Box(
        Modifier
            .pressable(shape = CircleShape, onClick = onClick)
            .semantics(mergeDescendants = true) {
                contentDescription = label
                this.selected = selected
            }
            .testTag(tag)
            .border(2.dp, if (selected) c.accent else Color.Transparent, CircleShape)
            .padding(4.dp),
        contentAlignment = Alignment.Center,
    ) { content() }
}

/** A palette colour as the blobatar draws its head. */
private fun headColor(colour: Blobatar.Colour): Color = Color(android.graphics.Color.parseColor(Blobatar.headHex(colour)))
