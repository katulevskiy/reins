package dev.rewarden.android.ui.settings

import android.content.ClipData
import android.content.ClipboardManager
import android.content.Context
import android.os.Build
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import dev.rewarden.android.design.ActionKind
import dev.rewarden.android.design.Banner
import dev.rewarden.android.design.BannerKind
import dev.rewarden.android.design.ButtonStyle
import dev.rewarden.android.design.CapsuleButton
import dev.rewarden.android.design.ConfirmDialog
import dev.rewarden.android.design.Glyph
import dev.rewarden.android.design.GlyphIcon
import dev.rewarden.android.design.Group
import dev.rewarden.android.design.Hairline
import dev.rewarden.android.design.ListRow
import dev.rewarden.android.design.LocalColors
import dev.rewarden.android.design.RText
import dev.rewarden.android.design.RType
import dev.rewarden.android.design.Screen
import dev.rewarden.android.design.pressable
import dev.rewarden.android.feedback.Event
import dev.rewarden.android.feedback.FeedbackSettings
import dev.rewarden.android.feedback.LocalFeedback
import dev.rewarden.android.feedback.play
import dev.rewarden.android.platform.FirebaseSupport
import dev.rewarden.android.state.AppState
import dev.rewarden.android.state.SessionState
import dev.rewarden.android.ui.common.ConnectionIcon
import dev.rewarden.android.ui.common.relativeTime
import dev.rewarden.android.ui.common.untrusted

@Composable
fun SettingsScreen(
    state: AppState,
    viewModel: SettingsViewModel,
    onBack: () -> Unit,
    onConnection: (String) -> Unit,
    onIntegrations: () -> Unit,
    onSounds: () -> Unit,
    onAutopilot: () -> Unit = {},
    onConnectComputer: () -> Unit = {},
) {
    val c = LocalColors.current
    val feedback = LocalFeedback.current
    val context = LocalContext.current
    val ui by viewModel.ui.collectAsStateWithLifecycle()
    val sounds by viewModel.sounds.collectAsStateWithLifecycle()
    val session by state.session.collectAsStateWithLifecycle()
    val connections by state.connections.collectAsStateWithLifecycle()
    val approvalDevice by state.approvalDevice.collectAsStateWithLifecycle()
    val accounts by state.accounts.collectAsStateWithLifecycle()
    val autopilot by state.autopilot.collectAsStateWithLifecycle()
    var confirmSignOut by remember { mutableStateOf(false) }
    val pushAvailable = FirebaseSupport.available(LocalContext.current)

    Screen(title = "Settings", onBack = onBack) {
        (session as? SessionState.SignedIn)?.let {
            Group(header = "Account") {
                ListRow("Email", subtitle = it.info.email, glyph = Glyph.Mail, ltrSubtitle = true)
                Hairline(inset = 51.dp)
                ListRow(
                    "Server",
                    Modifier.testTag("copyServer"),
                    subtitle = it.info.serverUrl,
                    glyph = Glyph.Link,
                    ltrSubtitle = true,
                    trailing = { GlyphIcon(Glyph.Copy, c.tertiary, size = 17.dp) },
                ) {
                    copyText(context, "Rewarden server", it.info.serverUrl)
                    feedback.play(Event.Copied)
                    // Android 13 and later confirm a copy themselves.
                    if (Build.VERSION.SDK_INT < Build.VERSION_CODES.TIRAMISU) viewModel.notice("Server address copied.")
                }
            }
        }

        Group(
            header = "Approval device",
            footer = when {
                approvalDevice && pushAvailable -> "Requests reach this phone by push notification and while the app is open."
                approvalDevice -> "Push notifications are not set up in this build. Requests arrive while the app is open."
                else -> "Only one phone at a time approves requests. Use this one to take over."
            },
        ) {
            Column(Modifier.padding(16.dp)) {
                if (approvalDevice) {
                    CapsuleButton(
                        "This phone is used for approvals",
                        Modifier.fillMaxWidth().testTag("registerPhone"),
                        style = ButtonStyle.Secondary,
                        enabled = false,
                        glyph = Glyph.Check,
                    ) {}
                } else {
                    CapsuleButton(
                        "Use this phone for approvals",
                        Modifier.fillMaxWidth().testTag("registerPhone"),
                        enabled = !ui.busy,
                        glyph = Glyph.Phone,
                        onClick = viewModel::registerThisPhone,
                    )
                }
            }
        }
        Column(Modifier.padding(horizontal = 16.dp)) {
            ui.message?.let { Banner(it, Modifier.padding(top = 10.dp)) }
            ui.error?.let { Banner(it, Modifier.padding(top = 10.dp), BannerKind.Error) }
        }

        Group(header = "Autopilot") {
            val mode = autopilot?.mode ?: dev.rewarden.core.AutopilotMode.MANUAL
            ListRow(
                "Autopilot",
                subtitle = autopilotSummary(autopilot),
                glyph = dev.rewarden.android.ui.autopilot.modeGlyph(mode),
                tint = dev.rewarden.android.ui.autopilot.modeTint(mode, c),
                chevron = true,
                modifier = Modifier.testTag("openAutopilot"),
                onClick = onAutopilot,
            )
        }

        Group(header = "AI connections") {
            if (connections.isEmpty()) {
                ListRow("No AI is connected yet", subtitle = "Add Rewarden to Claude or ChatGPT with your server's /mcp address.")
            }
            connections.forEachIndexed { i, connection ->
                if (i > 0) Hairline(inset = 68.dp)
                Row(
                    Modifier
                        .fillMaxWidth()
                        .testTag("connection:${connection.id}")
                        .pressable(highlight = c.controlFill, shape = RoundedCornerShape(0.dp)) { onConnection(connection.id) }
                        .padding(horizontal = 16.dp, vertical = 12.dp),
                    verticalAlignment = Alignment.CenterVertically,
                ) {
                    ConnectionIcon(connection.id, connection.label, size = 40.dp)
                    Spacer(Modifier.width(14.dp))
                    Column(Modifier.weight(1f)) {
                        RText(untrusted(connection.label), RType.sans(16f, FontWeight.Medium), c.text, maxLines = 1)
                        RText(
                            untrusted(connection.clientHost) + " · " + (connection.lastUsedAt?.let { "used ${relativeTime(it)}" } ?: "never used"),
                            RType.sans(13f),
                            c.secondary,
                            maxLines = 1,
                            ltr = true,
                        )
                    }
                    GlyphIcon(Glyph.ChevronRight, c.tertiary, size = 14.dp)
                }
            }
            Hairline(inset = if (connections.isEmpty()) 16.dp else 68.dp)
            ListRow(
                "Connect a computer",
                Modifier.testTag("connectComputer"),
                subtitle = "Scan the QR code the desktop app or rewarden login shows",
                glyph = Glyph.Qr,
                tint = c.accent,
                chevron = true,
                onClick = onConnectComputer,
            )
        }

        Group(header = "Integrations") {
            ListRow(
                "Integrations",
                subtitle = when (val n = accounts.size) {
                    0 -> "Connect Gmail and more"
                    1 -> "1 account connected"
                    else -> "$n accounts connected"
                },
                glyph = Glyph.Apps,
                tint = ActionKind.Read.tint(c),
                chevron = true,
                modifier = Modifier.testTag("openIntegrations"),
                onClick = onIntegrations,
            )
        }

        Group(header = "Sounds & haptics") {
            ListRow(
                "Sounds & haptics",
                subtitle = soundsSummary(sounds),
                glyph = Glyph.Speaker,
                tint = c.secondary,
                chevron = true,
                modifier = Modifier.testTag("openSounds"),
                onClick = onSounds,
            )
        }

        val updates = viewModel.updates
        if (updates != null) {
            val update by updates.state.collectAsStateWithLifecycle()
            UpdatesSection(
                update,
                onCheck = updates::checkNow,
                onInstall = updates::install,
                onRetry = updates::retry,
                onAutoDownload = updates::setAutoDownload,
            )
        } else {
            VersionSection()
        }

        Group(header = "Session") {
            Column(Modifier.padding(16.dp)) {
                CapsuleButton(
                    "Sign out",
                    Modifier.fillMaxWidth().testTag("signOut"),
                    style = ButtonStyle.Destructive,
                    enabled = !ui.busy,
                    glyph = Glyph.SignOut,
                    onClick = { confirmSignOut = true },
                )
            }
        }
        Spacer(Modifier.height(32.dp))
    }

    if (confirmSignOut) {
        ConfirmDialog(
            title = "Sign out?",
            text = "This phone stops receiving approval requests until you sign in again.",
            confirmLabel = "Sign out",
            onConfirm = {
                confirmSignOut = false
                viewModel.signOut()
            },
            onDismiss = { confirmSignOut = false },
        )
    }
}

/** What the Autopilot row says: the mode, and whether the model is here. */
internal fun autopilotSummary(s: dev.rewarden.core.AutopilotSettings?): String {
    if (s == null) return "Answers requests for you, on this phone"
    val mode = dev.rewarden.android.autopilot.AutopilotText.name(s.mode)
    val model = when (s.model.state) {
        dev.rewarden.core.ModelState.INSTALLED -> "model on this phone"
        dev.rewarden.core.ModelState.DOWNLOADING -> "downloading the model"
        else -> "no model yet"
    }
    return "$mode · $model"
}

/** What the Sounds & haptics row says about the switches behind it. */
internal fun soundsSummary(s: FeedbackSettings): String = when {
    !s.master -> "Off"
    s.soundsOn && s.hapticsOn -> "Sounds and haptics on"
    s.soundsOn -> "Sounds on, haptics off"
    s.hapticsOn -> "Haptics on, sounds off"
    else -> "Off"
}

private fun copyText(context: Context, label: String, text: String) {
    context.getSystemService(ClipboardManager::class.java)?.setPrimaryClip(ClipData.newPlainText(label, text))
}
