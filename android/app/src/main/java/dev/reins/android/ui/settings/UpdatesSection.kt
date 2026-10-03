package dev.rewarden.android.ui.settings

import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.width
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.unit.dp
import dev.rewarden.android.BuildConfig
import dev.rewarden.android.design.Banner
import dev.rewarden.android.design.BannerKind
import dev.rewarden.android.design.ButtonStyle
import dev.rewarden.android.design.CapsuleButton
import dev.rewarden.android.design.Glyph
import dev.rewarden.android.design.GlyphIcon
import dev.rewarden.android.design.Group
import dev.rewarden.android.design.Hairline
import dev.rewarden.android.design.ListRow
import dev.rewarden.android.design.LocalColors
import dev.rewarden.android.design.RText
import dev.rewarden.android.design.RType
import dev.rewarden.android.design.SwitchRow
import dev.rewarden.android.platform.update.UpdateState
import dev.rewarden.android.platform.update.UpdateStatus
import dev.rewarden.android.ui.update.UpdateProgressBar

/** Settings > Updates: the running version, "Check for updates" and its outcome, and automatic downloads. */
@Composable
fun UpdatesSection(
    state: UpdateState,
    onCheck: () -> Unit,
    onInstall: () -> Unit,
    onRetry: () -> Unit,
    onAutoDownload: (Boolean) -> Unit,
) {
    val c = LocalColors.current
    Group(header = "Updates") {
        VersionRow()
        Hairline(inset = 51.dp)
        SwitchRow(
            "Download updates automatically",
            state.autoDownload,
            onAutoDownload,
            subtitle = "You still choose when to install.",
            glyph = Glyph.Download,
            tag = "autoDownload",
        )
        Hairline(inset = 51.dp)
        Column(Modifier.padding(16.dp)) {
            val status = state.status
            when (status) {
                UpdateStatus.UpToDate -> StatusLine("Rewarden is up to date.", Glyph.Check, c.success)
                is UpdateStatus.Available -> StatusLine("Rewarden ${status.release.versionName} is available.", Glyph.Download, c.accent)
                is UpdateStatus.Downloading -> {
                    StatusLine("Downloading Rewarden ${status.release.versionName}… ${status.percent}%", Glyph.Download, c.accent)
                    UpdateProgressBar(status.percent, Modifier.padding(bottom = 14.dp).testTag("updateProgress"))
                }
                is UpdateStatus.Installing -> StatusLine("Confirm in Android's installer to finish.", Glyph.Info, c.secondary)
                is UpdateStatus.Failed -> Banner(status.message, Modifier.padding(bottom = 14.dp), BannerKind.Error, tag = "updateError")
                else -> Unit
            }
            when (status) {
                UpdateStatus.Idle, UpdateStatus.UpToDate, UpdateStatus.Checking -> CapsuleButton(
                    if (status == UpdateStatus.Checking) "Checking for updates…" else "Check for updates",
                    Modifier.fillMaxWidth().testTag("checkUpdates"),
                    style = ButtonStyle.Secondary,
                    busy = status == UpdateStatus.Checking,
                    glyph = Glyph.Refresh,
                    onClick = onCheck,
                )
                is UpdateStatus.Available -> CapsuleButton(
                    "Download and install",
                    Modifier.fillMaxWidth().testTag("installUpdate"),
                    style = ButtonStyle.Accent,
                    glyph = Glyph.Download,
                    onClick = onInstall,
                )
                is UpdateStatus.Ready -> CapsuleButton(
                    "Update ready: ${status.release.versionName} — Install",
                    Modifier.fillMaxWidth().testTag("installUpdate"),
                    style = ButtonStyle.Accent,
                    glyph = Glyph.Download,
                    maxLines = 2,
                    onClick = onInstall,
                )
                is UpdateStatus.Installing -> CapsuleButton(
                    "Installing…",
                    Modifier.fillMaxWidth().testTag("installUpdate"),
                    style = ButtonStyle.Secondary,
                    busy = true,
                ) {}
                is UpdateStatus.Failed -> CapsuleButton(
                    "Try again",
                    Modifier.fillMaxWidth().testTag("retryUpdate"),
                    style = ButtonStyle.Secondary,
                    glyph = Glyph.Refresh,
                    onClick = onRetry,
                )
                is UpdateStatus.Downloading -> Unit
            }
        }
    }
}

/** Settings > Version, when Google Play updates the app (the `play` build has no updater). */
@Composable
fun VersionSection() {
    Group(header = "Version", footer = "Google Play keeps Rewarden up to date.") {
        VersionRow()
    }
}

@Composable
private fun VersionRow() {
    ListRow(
        "Version",
        Modifier.testTag("appVersion"),
        subtitle = "${BuildConfig.VERSION_NAME} (${BuildConfig.BUILD_ID})",
        glyph = Glyph.Info,
        ltrSubtitle = true,
    )
}

@Composable
private fun StatusLine(text: String, glyph: Glyph, tint: androidx.compose.ui.graphics.Color) {
    val c = LocalColors.current
    Row(Modifier.padding(bottom = 12.dp).testTag("updateStatus"), verticalAlignment = Alignment.CenterVertically) {
        GlyphIcon(glyph, tint, size = 18.dp)
        Spacer(Modifier.width(8.dp))
        RText(text, RType.sans(14.5f), c.text)
    }
}
