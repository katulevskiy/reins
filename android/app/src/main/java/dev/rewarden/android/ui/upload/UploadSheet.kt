package dev.rewarden.android.ui.upload

import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.WindowInsets
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.navigationBars
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.layout.windowInsetsPadding
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.verticalScroll
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import dev.rewarden.android.design.ActionKind
import dev.rewarden.android.design.ActionTile
import dev.rewarden.android.design.Banner
import dev.rewarden.android.design.BannerKind
import dev.rewarden.android.design.ButtonStyle
import dev.rewarden.android.design.CapsuleButton
import dev.rewarden.android.design.ConnectionAvatar
import dev.rewarden.android.design.ConnectorTags
import dev.rewarden.android.design.LocalColors
import dev.rewarden.android.design.RText
import dev.rewarden.android.design.RType
import dev.rewarden.android.design.Spinner
import dev.rewarden.android.platform.Authenticator
import dev.rewarden.android.ui.common.FileCard
import dev.rewarden.android.ui.common.formatTime
import dev.rewarden.android.ui.common.untrusted

/** A file an AI uploaded through the server: who, what, why, and a preview, then approve or deny. */
@Composable
fun UploadSheet(viewModel: UploadViewModel, authenticator: Authenticator, onDone: () -> Unit) {
    val c = LocalColors.current
    val ui by viewModel.ui.collectAsStateWithLifecycle()
    LaunchedEffect(ui.finished) { if (ui.finished) onDone() }
    val view = ui.view
    if (view == null) {
        Column(Modifier.fillMaxWidth().padding(32.dp), horizontalAlignment = Alignment.CenterHorizontally) {
            if (ui.loading) Spinner(c.secondary, size = 32.dp)
            ui.error?.let {
                Banner(it, Modifier.padding(top = 32.dp), BannerKind.Error, tag = "uploadError")
                CapsuleButton("Close", Modifier.padding(top = 16.dp), style = ButtonStyle.Secondary, onClick = onDone)
            }
        }
        return
    }
    val who = untrusted(view.connectionLabel)
    Column(Modifier.fillMaxWidth().testTag("uploadSheet")) {
        Column(Modifier.weight(1f).verticalScroll(rememberScrollState())) {
            Column(Modifier.padding(start = 20.dp, end = 60.dp, top = 8.dp, bottom = 8.dp)) {
                Row(verticalAlignment = Alignment.CenterVertically) {
                    ConnectionAvatar(who, null, size = 40.dp)
                    Spacer(Modifier.width(12.dp))
                    Column(Modifier.weight(1f)) {
                        RText(who, RType.sans(17f, FontWeight.SemiBold), c.text, maxLines = 1)
                        RText(formatTime(view.createdAt), RType.sans(12.5f), c.tertiary, maxLines = 1)
                    }
                }
                Row(Modifier.padding(top = 16.dp), verticalAlignment = Alignment.CenterVertically) {
                    ActionTile(ActionKind.Upload, 1, size = 48.dp)
                    Spacer(Modifier.width(14.dp))
                    RText("Share a file", RType.sans(26f, FontWeight.SemiBold), c.text, Modifier.weight(1f).testTag("what"), maxLines = 2)
                }
                ConnectorTags("files", null, Modifier.padding(top = 12.dp))
            }
            if (view.purpose.isNotBlank()) {
                Column(
                    Modifier
                        .padding(horizontal = 16.dp, vertical = 8.dp)
                        .fillMaxWidth()
                        .background(c.controlFill, RoundedCornerShape(14.dp))
                        .padding(12.dp)
                        .testTag("uploadReason")
                        .semantics(mergeDescendants = true) {},
                ) {
                    RText("$who says it is for", RType.sans(12f, FontWeight.Medium), c.tertiary)
                    RText("“${untrusted(view.purpose)}”", RType.sans(15f, lineHeight = 21f), c.text, Modifier.padding(top = 2.dp))
                }
            }
            FileCard(view, Modifier.padding(horizontal = 16.dp, vertical = 8.dp))
            RText(
                "$who uploaded this file to your Rewarden server. If you approve, its download link works and $who can hand " +
                    "it to another tool. If you deny, the server deletes it. Either way it is gone at ${formatTime(view.expiresAt)}.",
                RType.sans(13.5f, lineHeight = 19f),
                c.secondary,
                Modifier.padding(horizontal = 20.dp, vertical = 10.dp),
            )
            ui.error?.let { Banner(it, Modifier.padding(16.dp), BannerKind.Error) }
        }
        Row(
            Modifier
                .fillMaxWidth()
                .background(c.background)
                .windowInsetsPadding(WindowInsets.navigationBars)
                .padding(horizontal = 16.dp, vertical = 12.dp),
            horizontalArrangement = Arrangement.spacedBy(12.dp),
        ) {
            CapsuleButton("Deny", Modifier.weight(1f).testTag("deny"), style = ButtonStyle.Secondary, enabled = !ui.busy, onClick = viewModel::deny)
            CapsuleButton("Approve", Modifier.weight(1.3f).testTag("approve"), enabled = !ui.busy, busy = ui.busy) {
                viewModel.approve(authenticator)
            }
        }
    }
}
