package dev.reins.android.ui.update

import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.ui.draw.shadow
import dev.reins.android.design.ButtonSize
import dev.reins.android.design.SignalButton
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
    // Signal (And-Core 05): a raised card just above the bar, a monochrome tile, and Later · Install beside the words.
    val shape = RoundedCornerShape(24.dp)
    Column(
        modifier
            .fillMaxWidth()
            .padding(horizontal = 14.dp)
            .shadow(24.dp, shape, ambientColor = androidx.compose.ui.graphics.Color(0x80000000), spotColor = androidx.compose.ui.graphics.Color(0x80000000))
            .clip(shape)
            .background(c.paperRaised)
            .border(1.dp, c.rule, shape)
            .padding(14.dp)
            .testTag("updatePrompt"),
    ) {
        Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(12.dp)) {
            Box(Modifier.size(38.dp).clip(RoundedCornerShape(11.dp)).background(if (status is UpdateStatus.Failed) c.dangerWash else c.paperSunken), contentAlignment = Alignment.Center) {
                when (status) {
                    is UpdateStatus.Installing -> Spinner(c.ink, size = 20.dp)
                    is UpdateStatus.Failed -> GlyphIcon(Glyph.Warning, c.danger, size = 20.dp)
                    else -> GlyphIcon(Glyph.Download, c.ink, size = 20.dp)
                }
            }
            Column(Modifier.weight(1f)) {
                RText(title, RType.row, c.ink, maxLines = 1)
                RText(body.removeSuffix("."), RType.meta, c.inkMeta, maxLines = 4)
            }
            if (status is UpdateStatus.Ready || status is UpdateStatus.Available || status is UpdateStatus.Failed) {
                SignalButton("Later", Modifier.testTag("updatePromptLater"), style = ButtonStyle.Soft, size = ButtonSize.Compact, onClick = onLater)
                if (status is UpdateStatus.Failed) {
                    SignalButton("Try again", Modifier.testTag("updatePromptRetry"), size = ButtonSize.Compact, onClick = onRetry)
                } else {
                    SignalButton("Install", Modifier.testTag("updatePromptInstall"), size = ButtonSize.Compact, onClick = onInstall)
                }
            }
        }
        if (status is UpdateStatus.Downloading) {
            UpdateProgressBar(status.percent, Modifier.padding(top = 12.dp))
        }
    }
}

/** Before Android's "install unknown apps" screen: why it is needed and what to turn on. */
@Composable
fun InstallPermissionDialog(onOpenSettings: () -> Unit, onDismiss: () -> Unit) {
    dev.reins.android.design.SignalDialog(
        title = "Allow updates",
        text = "Android asks once. Turn on “Allow from this source”.",
        confirmLabel = "Allow",
        onConfirm = onOpenSettings,
        onDismiss = onDismiss,
        italic = "updates",
        cancelLabel = "Not now",
        confirmTag = "allowInstalls",
    )
}

/** The update prompt for [updates]. [allowed] is false while something more important is on screen (an approval sheet). */
@Composable
fun UpdatePromptHost(updates: UpdateController, allowed: Boolean, modifier: Modifier = Modifier) {
    val state by updates.state.collectAsStateWithLifecycle()
    if (!allowed) return
    if (state.showPrompt) {
        UpdatePrompt(state.status, onInstall = updates::install, onLater = updates::later, onRetry = updates::retry, modifier = modifier)
    }
}

/**
 * The moment Install is tapped without Android's permission to install apps, wherever that was (the prompt, Settings >
 * Updates, the notification): a short dialog with a button to the setting; the install goes on when the user is back
 * with it allowed.
 */
@Composable
fun InstallPermissionHost(updates: UpdateController) {
    val state by updates.state.collectAsStateWithLifecycle()
    val context = LocalContext.current
    if (state.askPermission) {
        InstallPermissionDialog(onOpenSettings = { updates.openPermissionSettings(context) }, onDismiss = updates::dismissPermission)
    }
}
