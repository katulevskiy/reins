package dev.reins.android.ui.settings

import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.selection.toggleable
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.rounded.Download
import androidx.compose.material.icons.rounded.Info
import androidx.compose.material.icons.rounded.SystemUpdate
import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.unit.dp
import dev.reins.android.BuildConfig
import dev.reins.android.design.Banner
import dev.reins.android.design.BannerKind
import dev.reins.android.design.LocalColors
import dev.reins.android.design.RText
import dev.reins.android.design.RType
import dev.reins.android.design.Toggle
import dev.reins.android.feedback.LocalFeedback
import dev.reins.android.feedback.Event
import dev.reins.android.feedback.play
import dev.reins.android.platform.update.UpdateState
import dev.reins.android.platform.update.UpdateStatus
import dev.reins.android.ui.update.UpdateProgressBar

/** The running version, in mono on the right. */
@Composable
fun VersionRow() {
    SettingsRow(
        "Version",
        Modifier.testTag("appVersion"),
        icon = Icons.Rounded.Info,
        value = "${BuildConfig.VERSION_NAME} (${BuildConfig.BUILD_ID})",
        monoValue = true,
        chevron = false,
    )
}

/**
 * The in-app updater's rows under the version (sideloaded builds; Google Play updates the others): what it found and
 * the one thing to do about it, then whether updates download by themselves.
 */
@Composable
fun UpdateRows(
    state: UpdateState,
    onCheck: () -> Unit,
    onInstall: () -> Unit,
    onRetry: () -> Unit,
    onAutoDownload: (Boolean) -> Unit,
) {
    val c = LocalColors.current
    val feedback = LocalFeedback.current
    val status = state.status
    val line = updateLine(status)
    val (action, tag, onClick) = when (status) {
        UpdateStatus.Idle, UpdateStatus.UpToDate -> Triple("Check", "checkUpdates", onCheck)
        UpdateStatus.Checking -> Triple(null, "checkUpdates", null)
        is UpdateStatus.Available -> Triple("Install", "installUpdate", onInstall)
        is UpdateStatus.Ready -> Triple("Install", "installUpdate", onInstall)
        is UpdateStatus.Installing -> Triple(null, "installUpdate", null)
        is UpdateStatus.Downloading -> Triple("${status.percent}%", "updateDownloading", null)
        is UpdateStatus.Failed -> Triple("Try again", "retryUpdate", onRetry)
    }
    SettingsRow(
        "Updates",
        Modifier.testTag(tag),
        icon = Icons.Rounded.SystemUpdate,
        subtitle = line,
        subtitleTag = "updateStatus",
        value = action,
        busy = status == UpdateStatus.Checking || status is UpdateStatus.Installing,
        chevron = false,
        valueContent = if (action != null && onClick != null) {
            { RText(action, RType.sans(15f, androidx.compose.ui.text.font.FontWeight.SemiBold), c.accent, maxLines = 1) }
        } else null,
        onClick = onClick,
    )
    if (status is UpdateStatus.Downloading) {
        UpdateProgressBar(status.percent, Modifier.padding(start = 70.dp, end = 16.dp, bottom = 12.dp).testTag("updateProgress"))
    }
    if (status is UpdateStatus.Failed) {
        Banner(status.message, Modifier.padding(start = 16.dp, end = 16.dp, bottom = 12.dp), BannerKind.Error, tag = "updateError")
    }
    SettingsRow(
        "Download automatically",
        Modifier
            .testTag("autoDownload")
            .toggleable(value = state.autoDownload, role = Role.Switch) {
                onAutoDownload(it)
                feedback.play(Event.toggle(it))
            },
        icon = Icons.Rounded.Download,
        chevron = false,
        valueContent = {
            Spacer(Modifier.width(2.dp))
            Toggle(state.autoDownload, onChange = onAutoDownload)
        },
    )
}

/** What the updater found, in a few words; null before the first check. */
internal fun updateLine(status: UpdateStatus): String? = when (status) {
    UpdateStatus.Idle -> null
    UpdateStatus.Checking -> "Checking…"
    UpdateStatus.UpToDate -> "Up to date"
    is UpdateStatus.Available -> "${status.release.versionName} is available"
    is UpdateStatus.Downloading -> "Downloading ${status.release.versionName}"
    is UpdateStatus.Ready -> "${status.release.versionName} is ready"
    is UpdateStatus.Installing -> "Confirm in the installer"
    is UpdateStatus.Failed -> "Did not work"
}
