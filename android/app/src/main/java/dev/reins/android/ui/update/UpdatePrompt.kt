package dev.reins.android.ui.update

import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxHeight
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import androidx.compose.ui.window.Dialog
import androidx.compose.ui.window.DialogProperties
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import dev.reins.android.feedback.DialogFeedback
import dev.reins.android.design.ButtonStyle
import dev.reins.android.design.CapsuleButton
import dev.reins.android.design.Glyph
import dev.reins.android.design.GlyphIcon
import dev.reins.android.design.LocalColors
import dev.reins.android.design.RText
import dev.reins.android.design.RType
import dev.reins.android.design.Spinner
import dev.reins.android.design.glass
import dev.reins.android.platform.update.UpdateController
import dev.reins.android.platform.update.UpdateStatus

/** A thin bar for download progress (0..100). */
@Composable
fun UpdateProgressBar(percent: Int, modifier: Modifier = Modifier) {
    val c = LocalColors.current
    Box(modifier.fillMaxWidth().height(6.dp).clip(CircleShape).background(c.controlFill)) {
        Box(Modifier.fillMaxHeight().fillMaxWidth(percent.coerceIn(0, 100) / 100f).clip(CircleShape).background(c.accent))
    }
}

/**
 * The floating "update ready" card above the tab bar. It never blocks anything: the app stays usable around it, and
 * the caller hides it while an approval sheet is up.
 */
@Composable
fun UpdatePrompt(status: UpdateStatus, onInstall: () -> Unit, onLater: () -> Unit, onRetry: () -> Unit, modifier: Modifier = Modifier) {
    val c = LocalColors.current
    val release = status.release ?: return
    val name = release.versionName
    val (title, body) = when (status) {
        is UpdateStatus.Ready -> "Update ready" to "Reins $name is ready to install."
        is UpdateStatus.Available -> "Update available" to "Reins $name is available."
        is UpdateStatus.Downloading -> "Downloading update" to "Reins $name · ${status.percent}%"
        is UpdateStatus.Installing -> "Installing update" to "Confirm in Android's installer to finish."
        is UpdateStatus.Failed -> "Update failed" to status.message
        else -> return
    }
    Column(
        modifier
            .fillMaxWidth()
            .padding(horizontal = 16.dp)
            .glass(c, RoundedCornerShape(24.dp), 10.dp)
            .padding(16.dp)
            .testTag("updatePrompt"),
    ) {
        Row(verticalAlignment = Alignment.CenterVertically) {
            Box(Modifier.size(40.dp).clip(CircleShape).background(if (status is UpdateStatus.Failed) c.danger.copy(alpha = 0.12f) else c.accentSoft), contentAlignment = Alignment.Center) {
                when (status) {
                    is UpdateStatus.Installing -> Spinner(c.accent, size = 22.dp)
                    is UpdateStatus.Failed -> GlyphIcon(Glyph.Warning, c.danger, size = 20.dp)
                    else -> GlyphIcon(Glyph.Download, c.accent, size = 20.dp)
                }
            }
            Spacer(Modifier.width(12.dp))
            Column(Modifier.weight(1f)) {
                RText(title, RType.sans(16f, FontWeight.SemiBold), c.text, maxLines = 1)
                RText(body, RType.sans(14f, lineHeight = 19f), c.secondary, Modifier.padding(top = 2.dp), maxLines = 4)
            }
        }
        if (status is UpdateStatus.Downloading) {
            UpdateProgressBar(status.percent, Modifier.padding(top = 14.dp))
        }
        if (status is UpdateStatus.Ready || status is UpdateStatus.Available || status is UpdateStatus.Failed) {
            Row(Modifier.fillMaxWidth().padding(top = 14.dp), horizontalArrangement = Arrangement.spacedBy(10.dp)) {
                CapsuleButton("Later", Modifier.weight(1f).testTag("updatePromptLater"), style = ButtonStyle.Secondary, compact = true, onClick = onLater)
                if (status is UpdateStatus.Failed) {
                    CapsuleButton("Try again", Modifier.weight(1f).testTag("updatePromptRetry"), style = ButtonStyle.Accent, compact = true, onClick = onRetry)
                } else {
                    CapsuleButton("Install", Modifier.weight(1f).testTag("updatePromptInstall"), style = ButtonStyle.Accent, compact = true, glyph = Glyph.Download, onClick = onInstall)
                }
            }
        }
    }
}

/** Before Android's "install unknown apps" screen: why it is needed and what to turn on. */
@Composable
fun InstallPermissionDialog(onOpenSettings: () -> Unit, onDismiss: () -> Unit) {
    val c = LocalColors.current
    Dialog(onDismissRequest = onDismiss, properties = DialogProperties(usePlatformDefaultWidth = false)) {
        DialogFeedback()
        Column(Modifier.padding(horizontal = 28.dp).fillMaxWidth().glass(c, RoundedCornerShape(26.dp), 16.dp).padding(22.dp)) {
            RText("Allow Reins to install updates", RType.sans(19f, FontWeight.SemiBold), c.text)
            Spacer(Modifier.height(8.dp))
            RText(
                "Reins isn't installed from the Play Store, so Android asks once before it may install its own " +
                    "updates. Turn on “Allow from this source”, then come back to finish the update.",
                RType.sans(15f, lineHeight = 21f),
                c.secondary,
            )
            Spacer(Modifier.height(22.dp))
            Row(horizontalArrangement = Arrangement.spacedBy(10.dp)) {
                CapsuleButton("Not now", Modifier.weight(1f), style = ButtonStyle.Secondary, onClick = onDismiss)
                CapsuleButton("Open settings", Modifier.weight(1f).testTag("allowInstalls"), style = ButtonStyle.Accent, onClick = onOpenSettings)
            }
        }
    }
}

/**
 * The prompt and the install-permission explanation for [updates]. [allowed] is false while something more important is
 * on screen (an approval sheet); both then wait.
 */
@Composable
fun UpdatePromptHost(updates: UpdateController, allowed: Boolean, modifier: Modifier = Modifier) {
    val state by updates.state.collectAsStateWithLifecycle()
    val context = LocalContext.current
    if (!allowed) return
    if (state.showPrompt) {
        UpdatePrompt(state.status, onInstall = updates::install, onLater = updates::later, onRetry = updates::retry, modifier = modifier)
    }
    if (state.askPermission) {
        InstallPermissionDialog(onOpenSettings = { updates.openPermissionSettings(context) }, onDismiss = updates::dismissPermission)
    }
}
