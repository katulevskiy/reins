package dev.reins.android.ui.upload

import dev.reins.android.ui.approval.AskField
import dev.reins.android.ui.approval.AskWho
import dev.reins.android.ui.approval.AskDock
import dev.reins.android.ui.approval.AskButtons
import dev.reins.android.ui.approval.AskLoading
import dev.reins.android.ui.approval.FieldVerdict
import dev.reins.android.design.Field
import dev.reins.android.design.Glyph
import dev.reins.android.design.GlyphIcon
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.Box
import androidx.compose.ui.draw.clip

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
import dev.reins.android.design.ActionKind
import dev.reins.android.design.ActionTile
import dev.reins.android.design.Banner
import dev.reins.android.design.BannerKind
import dev.reins.android.design.ButtonStyle
import dev.reins.android.design.CapsuleButton
import dev.reins.android.design.ConnectionAvatar
import dev.reins.android.design.ConnectorTags
import dev.reins.android.design.LocalColors
import dev.reins.android.design.RText
import dev.reins.android.design.RType
import dev.reins.android.design.Spinner
import dev.reins.android.platform.Authenticator
import dev.reins.android.ui.common.FileCard
import dev.reins.android.ui.common.formatTime
import dev.reins.android.ui.common.quotedName
import dev.reins.android.ui.common.untrusted

/**
 * A file an AI uploaded through the server (Reins Signal, iOS-Connect 05): the orange share field with the AI's
 * purpose in its own words where the verdict usually is, then the file itself (its first lines, the image, or a kind
 * line) and what approving does. [onClose] leaves it waiting.
 */
@Composable
fun UploadSheet(viewModel: UploadViewModel, authenticator: Authenticator, onDone: () -> Unit, onClose: () -> Unit = onDone) {
    val c = LocalColors.current
    val ui by viewModel.ui.collectAsStateWithLifecycle()
    LaunchedEffect(ui.finished) { if (ui.finished) onDone() }
    val view = ui.view
    if (view == null) {
        AskLoading(ui.loading, ui.error, onClose, errorTag = "uploadError")
        return
    }
    // The AI names itself and the file: Reins' own words go in the ask, the AI's name only quoted on one short line.
    val who = quotedName(view.connectionLabel)
    Column(Modifier.fillMaxSize().background(c.paper).testTag("uploadSheet")) {
        Column(Modifier.weight(1f).verticalScroll(rememberScrollState())) {
            AskField(
                Field.Push,
                "Files · Share",
                "Share a file",
                "file",
                onClose = onClose,
                closeGlyph = Glyph.Close,
                closeLabel = "Close",
                top = {
                    AskWho(who, null) { ConnectionAvatar(who, null, size = 28.dp) }
                    RText(formatTime(view.createdAt), RType.mono(13f), c.onField, maxLines = 1)
                },
                // The AI's reason, quoted: its words, not Reins'.
                verdict = if (view.purpose.isNotBlank()) {
                    { FieldVerdict("“${untrusted(view.purpose)}”", Modifier.testTag("uploadReason"), maxLines = 3) }
                } else {
                    null
                },
            )
            FileCard(view, Modifier.padding(horizontal = 22.dp).padding(top = 22.dp))
            RText(
                "If you approve, its download link works and $who can hand it to another tool. Either way it is deleted at ${formatTime(view.expiresAt)}.",
                RType.meta,
                c.inkMeta,
                Modifier.padding(horizontal = 22.dp, vertical = 14.dp),
            )
            ui.error?.let { Banner(it, Modifier.padding(horizontal = 22.dp), BannerKind.Error) }
        }
        AskDock { AskButtons("Approve", onDeny = viewModel::deny, onApprove = { viewModel.approve(authenticator) }, busy = ui.busy) }
    }
}
