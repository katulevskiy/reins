package dev.reins.android.ui.circles

import android.content.ClipboardManager
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.ColumnScope
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.rounded.Block
import androidx.compose.material.icons.rounded.ContentPaste
import androidx.compose.material.icons.rounded.Handshake
import androidx.compose.material.icons.rounded.QrCodeScanner
import androidx.compose.material.icons.rounded.Visibility
import androidx.compose.material3.Icon
import androidx.compose.runtime.Composable
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.dp
import dev.reins.android.design.BlobAvatar
import dev.reins.android.design.ButtonSize
import dev.reins.android.design.ButtonStyle
import dev.reins.android.design.LocalColors
import dev.reins.android.design.RText
import dev.reins.android.design.RTextField
import dev.reins.android.design.RType
import dev.reins.android.design.SignalButton
import dev.reins.android.design.Spinner
import dev.reins.android.platform.QrScannerProvider
import dev.reins.android.platform.ScanResult
import dev.reins.android.ui.common.untrusted
import dev.reins.core.CircleInviteView
import dev.reins.core.CircleJoinStage
import kotlinx.coroutines.launch

/**
 * Join someone's Circle (Decisions v2, 04 §3, the helper's side): their link, pasted or scanned; then what you'd do for
 * them (the role, what it covers, how fast, whether you see the whole card); once their phone answers, the six digits
 * to compare with theirs, then Accept or Decline. Their phone confirms last.
 */
@Composable
fun JoinCircleSheet(
    flow: JoinFlow,
    onLink: (String) -> Unit,
    onOpen: () -> Unit,
    onAnswer: (Boolean) -> Unit,
    onClose: () -> Unit,
) {
    val view = flow.view
    CircleSheet(
        title = when (view?.stage) {
            null -> "Join a Circle"
            CircleJoinStage.COMPARE -> "Compare the code"
            else -> "${shortName(untrusted(view.inviter))}'s Circle"
        },
        tag = "joinCircleSheet",
        onBack = null,
        onDismiss = onClose,
        onClose = onClose,
    ) {
        if (view == null) LinkPage(flow, onLink, onOpen) else TermsPage(view, flow.busy, onAnswer, onClose)
        flow.error?.let { RText(it, RType.sans(14f, FontWeight.Medium), LocalColors.current.danger, Modifier.padding(top = 10.dp).testTag("joinCircleError"), maxLines = 3) }
    }
}

@Composable
private fun LinkPage(flow: JoinFlow, onLink: (String) -> Unit, onOpen: () -> Unit) {
    val c = LocalColors.current
    val context = LocalContext.current
    val scope = rememberCoroutineScope()
    RTextField(flow.link, onLink, "reins-circle:…", tag = "joinLinkField", enabled = !flow.busy, mono = true)
    Row(Modifier.fillMaxWidth().padding(top = 10.dp), horizontalArrangement = Arrangement.spacedBy(10.dp)) {
        SignalButton(
            "Paste",
            Modifier.weight(1f).testTag("joinPaste"),
            style = ButtonStyle.Soft,
            size = ButtonSize.Small,
            leading = { Icon(Icons.Rounded.ContentPaste, null, Modifier.size(18.dp), tint = c.ink) },
        ) {
            context.getSystemService(ClipboardManager::class.java)?.primaryClip?.takeIf { it.itemCount > 0 }
                ?.getItemAt(0)?.coerceToText(context)?.toString()?.let(onLink)
        }
        SignalButton(
            "Scan",
            Modifier.weight(1f).testTag("joinScan"),
            style = ButtonStyle.Soft,
            size = ButtonSize.Small,
            leading = { Icon(Icons.Rounded.QrCodeScanner, null, Modifier.size(18.dp), tint = c.ink) },
        ) {
            scope.launch {
                when (val result = QrScannerProvider.factory(context).scan()) {
                    is ScanResult.Scanned -> {
                        onLink(result.text)
                        if (circleLink(result.text) != null) onOpen()
                    }
                    else -> Unit
                }
            }
        }
    }
    Spacer(Modifier.size(16.dp))
    SignalButton(
        "Open",
        Modifier.fillMaxWidth().testTag("joinOpen"),
        enabled = !flow.busy && flow.link.isNotBlank(),
        leading = { if (flow.busy) Spinner(c.onInverse, size = 18.dp) else Icon(Icons.Rounded.Handshake, null, Modifier.size(20.dp), tint = c.onInverse) },
        onClick = onOpen,
    )
}

/** What the inviter asks of you, and where it stands. */
@Composable
private fun ColumnScope.TermsPage(view: CircleInviteView, busy: Boolean, onAnswer: (Boolean) -> Unit, onClose: () -> Unit) {
    val c = LocalColors.current
    Row(Modifier.fillMaxWidth().padding(bottom = 8.dp), horizontalArrangement = Arrangement.spacedBy(10.dp), verticalAlignment = Alignment.CenterVertically) {
        BlobAvatar(view.inviter, size = 28.dp)
        RText(untrusted(view.inviter), RType.sans(15f, FontWeight.Medium), c.ink, Modifier.weight(1f).testTag("joinInviter"), maxLines = 1)
    }
    Fact(roleIcon(view.role), roleLabel(view.role), "joinRole")
    untrusted(view.scopeLine).ifBlank { null }?.let { Fact(Icons.Rounded.Handshake, it, "joinScope") }
    if (decides(view.role)) {
        Fact(
            Icons.Rounded.Visibility,
            (if (view.fullCards) "Whole card" else "Summary") + " · " + classLine(view.deferred, view.windowSecs.toLong()),
            "joinClass",
        )
    }
    Spacer(Modifier.size(8.dp))
    when (view.stage) {
        CircleJoinStage.OPENED -> Waiting("Waiting for ${shortName(untrusted(view.inviter))}'s phone", "joinWaiting")
        CircleJoinStage.COMPARE -> {
            view.code?.let { BigCode(it, Modifier.align(Alignment.CenterHorizontally)) }
            RText(
                "Same on their phone?",
                RType.sans(14.5f),
                c.ink2,
                Modifier.fillMaxWidth().padding(vertical = 8.dp),
                align = TextAlign.Center,
            )
            Row(Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.spacedBy(10.dp)) {
                SignalButton(
                    "Decline",
                    Modifier.weight(1f).testTag("joinDecline"),
                    style = ButtonStyle.Outline,
                    enabled = !busy,
                    leading = { Icon(Icons.Rounded.Block, null, Modifier.size(18.dp), tint = c.ink) },
                ) { onAnswer(false) }
                SignalButton(
                    "Accept",
                    Modifier.weight(1f).testTag("joinAccept"),
                    enabled = !busy,
                    leading = { if (busy) Spinner(c.onInverse, size = 18.dp) else Icon(Icons.Rounded.Handshake, null, Modifier.size(18.dp), tint = c.onInverse) },
                ) { onAnswer(true) }
            }
        }
        CircleJoinStage.ACCEPTED -> {
            view.code?.let { BigCode(it, Modifier.align(Alignment.CenterHorizontally)) }
            Waiting("Waiting for them to confirm", "joinAccepted")
        }
        CircleJoinStage.ACTIVE -> {
            RText("You're in.", RType.sans(17f, FontWeight.Medium), c.ink, Modifier.padding(vertical = 10.dp).testTag("joinActive"))
            SignalButton("Done", Modifier.fillMaxWidth().testTag("joinDone"), onClick = onClose)
        }
        CircleJoinStage.DECLINED, CircleJoinStage.EXPIRED -> {
            RText(
                if (view.stage == CircleJoinStage.DECLINED) "Declined" else "Expired",
                RType.sans(17f, FontWeight.Medium),
                c.ink2,
                Modifier.padding(vertical = 10.dp).testTag("joinEnded"),
            )
            SignalButton("Close", Modifier.fillMaxWidth(), style = ButtonStyle.Outline, onClick = onClose)
        }
    }
}

@Composable
private fun Fact(icon: ImageVector, text: String, tag: String) {
    val c = LocalColors.current
    Row(Modifier.fillMaxWidth().padding(vertical = 5.dp), horizontalArrangement = Arrangement.spacedBy(10.dp), verticalAlignment = Alignment.CenterVertically) {
        Icon(icon, null, Modifier.size(18.dp), tint = c.ink2)
        RText(text, RType.sans(15f), c.ink, Modifier.weight(1f).testTag(tag), maxLines = 2)
    }
}

@Composable
private fun ColumnScope.Waiting(text: String, tag: String) {
    val c = LocalColors.current
    Row(Modifier.align(Alignment.CenterHorizontally).padding(vertical = 12.dp), horizontalArrangement = Arrangement.spacedBy(8.dp), verticalAlignment = Alignment.CenterVertically) {
        Spinner(c.inkMeta, size = 16.dp)
        RText(text, RType.sans(14f), c.inkMeta, Modifier.testTag(tag), maxLines = 1)
    }
}
