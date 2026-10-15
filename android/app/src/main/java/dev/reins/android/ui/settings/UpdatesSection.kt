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

import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.RowScope
import androidx.compose.foundation.layout.defaultMinSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.ui.Alignment
import androidx.compose.ui.text.font.FontWeight
import dev.reins.android.design.Glyph
import dev.reins.android.design.GlyphIcon
import dev.reins.android.design.Spinner
import dev.reins.android.design.hairlineBelow
import dev.reins.android.design.pressable

/** A row below Settings' fold (Signal): a monochrome tile, the title and its line, and what is on the right. */
@Composable
private fun FoldRow(title: String, glyph: Glyph, modifier: Modifier = Modifier, subtitle: String? = null, subtitleTag: String? = null, onClick: (() -> Unit)? = null, end: @Composable RowScope.() -> Unit = {}) {
    val c = LocalColors.current
    Row(
        modifier
            .fillMaxWidth()
            .defaultMinSize(minHeight = 56.dp)
            .hairlineBelow(c.rule)
            .then(if (onClick != null) Modifier.pressable(highlight = c.paperSunken, shape = RoundedCornerShape(0.dp), onClick = onClick) else Modifier)
            .padding(vertical = 8.dp),
        horizontalArrangement = Arrangement.spacedBy(12.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Box(Modifier.size(32.dp).background(c.paperSunken, RoundedCornerShape(10.dp)), contentAlignment = Alignment.Center) { GlyphIcon(glyph, c.ink, size = 18.dp) }
        Column(Modifier.weight(1f)) {
            RText(title, RType.sans(15f, FontWeight.Medium), c.ink, maxLines = 1)
            if (subtitle != null) RText(subtitle, RType.meta, c.inkMeta, Modifier.testTag(subtitleTag ?: "rowDetail"), maxLines = 1)
        }
        end()
    }
}

/** The running version, in mono on the right. */
@Composable
fun VersionRow() {
    val c = LocalColors.current
    FoldRow("Version", Glyph.Info, Modifier.testTag("appVersion")) {
        RText("${BuildConfig.VERSION_NAME} (${BuildConfig.BUILD_ID})", RType.mono(13.5f), c.inkMeta, maxLines = 1, ltr = true)
    }
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
    FoldRow("Updates", Glyph.Download, Modifier.testTag(tag), subtitle = line, subtitleTag = "updateStatus", onClick = onClick) {
        when {
            status == UpdateStatus.Checking || status is UpdateStatus.Installing -> Spinner(c.inkMeta, size = 18.dp)
            action != null -> RText(action, RType.sans(14.5f, FontWeight.SemiBold), if (onClick != null) c.violet else c.inkMeta, maxLines = 1)
        }
    }
    if (status is UpdateStatus.Downloading) {
        UpdateProgressBar(status.percent, Modifier.padding(start = 44.dp, top = 8.dp, bottom = 8.dp).testTag("updateProgress"))
    }
    if (status is UpdateStatus.Failed) {
        Banner(status.message, Modifier.padding(vertical = 8.dp), BannerKind.Error, tag = "updateError")
    }
    FoldRow(
        "Download automatically",
        Glyph.Wifi,
        Modifier
            .testTag("autoDownload")
            .toggleable(value = state.autoDownload, role = Role.Switch) {
                onAutoDownload(it)
                feedback.play(Event.toggle(it))
            },
    ) { Toggle(state.autoDownload, onChange = onAutoDownload) }
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
