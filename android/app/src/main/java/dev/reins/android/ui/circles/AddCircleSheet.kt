package dev.reins.android.ui.circles

import android.content.ClipData
import android.content.ClipboardManager
import android.content.Context
import android.content.Intent
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.ColumnScope
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.WindowInsets
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.imePadding
import androidx.compose.foundation.layout.navigationBars
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.layout.windowInsetsPadding
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.rounded.Bolt
import androidx.compose.material.icons.rounded.ContentCopy
import androidx.compose.material.icons.rounded.QrCode2
import androidx.compose.material.icons.rounded.Schedule
import androidx.compose.material.icons.rounded.Share
import androidx.compose.material3.Icon
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import androidx.compose.ui.window.Dialog
import androidx.compose.ui.window.DialogProperties
import dev.reins.android.design.AskText
import dev.reins.android.design.BlobAvatar
import dev.reins.android.design.ButtonStyle
import dev.reins.android.design.CircleIconButton
import dev.reins.android.design.Glyph
import dev.reins.android.design.HelpButton
import dev.reins.android.design.HelpText
import dev.reins.android.design.HoldButton
import dev.reins.android.design.LocalColors
import dev.reins.android.design.QrCode
import dev.reins.android.design.RText
import dev.reins.android.design.RType
import dev.reins.android.design.SignalButton
import dev.reins.android.design.Spinner
import dev.reins.android.design.pressable
import dev.reins.android.feedback.DialogFeedback
import dev.reins.android.ui.common.untrusted
import dev.reins.core.CircleInviteProgress
import dev.reins.core.CircleRole
import dev.reins.core.ConnectionView

/**
 * Add someone (Decisions v2, 04 §3): the role, what it covers, Instant or Deferred, then the invitation as a link and a
 * QR code. Once they open it both phones show the same six digits; once they accept, the owner holds to confirm (and
 * the phone's own check), and they're in.
 */
@Composable
fun AddCircleSheet(
    flow: AddFlow,
    connections: List<ConnectionView>,
    onRole: (CircleRole) -> Unit,
    onPreset: (String) -> Unit,
    onAgent: (String, String) -> Unit,
    onDeferred: (Boolean) -> Unit,
    onCreate: () -> Unit,
    onConfirm: () -> Unit,
    onBack: () -> Unit,
    onClose: () -> Unit,
) {
    val page = flow.page
    val back = page == AddPage.Scope || page == AddPage.Class
    CircleSheet(
        title = when (page) {
            AddPage.Role -> "Add someone"
            AddPage.Scope -> "What it covers"
            AddPage.Class -> "How fast"
            AddPage.Invite -> if (inviteCode(flow.progress) != null) "Compare the code" else "Send the link"
        },
        tag = "addCircleSheet",
        onBack = if (back) onBack else null,
        onDismiss = onBack,
        onClose = onClose,
    ) {
        when (page) {
            AddPage.Role -> RolePage(onRole)
            AddPage.Scope -> ScopePage(flow, connections, onPreset, onAgent)
            AddPage.Class -> ClassPage(flow, onDeferred, onCreate)
            AddPage.Invite -> InvitePage(flow, onConfirm, onClose)
        }
        flow.error?.let { RText(it, RType.sans(14f, FontWeight.Medium), LocalColors.current.danger, Modifier.padding(top = 10.dp).testTag("addCircleError"), maxLines = 3) }
    }
}

@Composable
private fun RolePage(onRole: (CircleRole) -> Unit) {
    CircleRole.entries.forEach { role ->
        OptionRow(roleLabel(role), roleHelp(role), roleIcon(role), "role:${role.name.lowercase()}") { onRole(role) }
    }
}

@Composable
private fun ScopePage(flow: AddFlow, connections: List<ConnectionView>, onPreset: (String) -> Unit, onAgent: (String, String) -> Unit) {
    val c = LocalColors.current
    if (flow.preset == "agent") {
        // One AI: which one.
        val ais = connections.filter { it.keyFingerprint == null }
        if (ais.isEmpty()) {
            RText("No AI is connected yet.", RType.sans(15f), c.ink2, Modifier.padding(vertical = 12.dp).testTag("noAgents"))
        }
        ais.forEach { conn ->
            OptionRow(untrusted(conn.label), null, null, "agent:${conn.id}", leading = { dev.reins.android.ui.common.ConnectionIcon(conn.id, conn.label, size = 28.dp) }) {
                onAgent(conn.id, conn.label)
            }
        }
        return
    }
    PRESETS.forEach { key ->
        OptionRow(presetLabel(key), null, null, "preset:$key", chevron = key == "agent") { onPreset(key) }
    }
}

@Composable
private fun ClassPage(flow: AddFlow, onDeferred: (Boolean) -> Unit, onCreate: () -> Unit) {
    val c = LocalColors.current
    val role = flow.role ?: return
    var help by rememberSaveable { mutableStateOf(false) }
    // Summary of what was picked.
    Row(Modifier.fillMaxWidth().padding(bottom = 10.dp), horizontalArrangement = Arrangement.spacedBy(8.dp), verticalAlignment = Alignment.CenterVertically) {
        Icon(roleIcon(role), null, Modifier.size(18.dp), tint = c.ink2)
        val scope = if (flow.preset == "agent") untrusted(flow.agentLabel) else presetLabel(flow.preset.orEmpty())
        RText("${roleLabel(role)} · $scope", RType.sans(15f, FontWeight.Medium), c.ink2, Modifier.weight(1f).testTag("addSummary"), maxLines = 2)
    }
    if (decides(role)) {
        Row(verticalAlignment = Alignment.CenterVertically) {
            Row(Modifier.weight(1f), horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                Segment("Deferred", Icons.Rounded.Schedule, flow.deferred, "classDeferred", Modifier.weight(1f)) { onDeferred(true) }
                Segment("Instant", Icons.Rounded.Bolt, !flow.deferred, "classInstant", Modifier.weight(1f)) { onDeferred(false) }
            }
            HelpButton(help, "Instant and Deferred") { help = it }
        }
        HelpText(
            "Deferred: they have ${windowLabel(DEFERRED_SECS)} to answer, and your agent carries on meanwhile. " +
                "Instant: they're at hand, and have ${windowLabel(INSTANT_SECS)}.",
            help,
            Modifier.padding(top = 6.dp),
        )
    }
    Spacer(Modifier.size(18.dp))
    SignalButton(
        "Create link",
        Modifier.fillMaxWidth().testTag("circleCreate"),
        enabled = !flow.busy,
        leading = { if (flow.busy) Spinner(c.onInverse, size = 18.dp) else Icon(Icons.Rounded.QrCode2, null, Modifier.size(20.dp), tint = c.onInverse) },
        onClick = onCreate,
    )
}

/** The invitation: the link and its QR code while nobody opened it; then the code; then Confirm. */
@Composable
private fun ColumnScope.InvitePage(flow: AddFlow, onConfirm: () -> Unit, onClose: () -> Unit) {
    val c = LocalColors.current
    val context = LocalContext.current
    val invite = flow.invite ?: return
    val progress = flow.progress
    val code = inviteCode(progress)
    val helper = inviteHelper(progress)
    val ended = inviteEnd(progress)
    when {
        flow.added != null || progress is CircleInviteProgress.Active -> {
            RText(
                "${flow.added?.peer?.let { shortName(untrusted(it)) } ?: "They"} ${if (flow.added != null) "can help now." else "joined."}",
                RType.sans(17f, FontWeight.Medium),
                c.ink,
                Modifier.padding(vertical = 10.dp).testTag("circleAdded"),
            )
            SignalButton("Done", Modifier.fillMaxWidth().testTag("circleDone"), onClick = onClose)
        }
        ended != null -> {
            RText(ended, RType.sans(17f, FontWeight.Medium), c.ink2, Modifier.padding(vertical = 10.dp).testTag("circleInviteEnded"))
            SignalButton("Close", Modifier.fillMaxWidth(), style = ButtonStyle.Outline, onClick = onClose)
        }
        code != null && helper != null -> {
            Row(Modifier.fillMaxWidth().padding(bottom = 6.dp), horizontalArrangement = Arrangement.spacedBy(10.dp), verticalAlignment = Alignment.CenterVertically) {
                BlobAvatar(helper, size = 28.dp)
                RText(untrusted(helper), RType.sans(15f, FontWeight.Medium), c.ink, Modifier.weight(1f).testTag("circleHelper"), maxLines = 1)
            }
            BigCode(code, Modifier.align(Alignment.CenterHorizontally))
            RText(
                if (progress is CircleInviteProgress.Accepted) "Same on their phone? Confirm." else "Same on their phone? Waiting for them to accept.",
                RType.sans(14.5f),
                c.ink2,
                Modifier.fillMaxWidth().padding(vertical = 10.dp).testTag("circleCompareLine"),
                align = TextAlign.Center,
            )
            if (progress is CircleInviteProgress.Accepted) {
                HoldButton(
                    "Hold to confirm",
                    c.violet,
                    Modifier.fillMaxWidth().testTag("circleConfirm"),
                    enabled = !flow.busy,
                    busy = flow.busy,
                    onConfirm = onConfirm,
                )
                Spacer(Modifier.size(10.dp))
            } else {
                Row(Modifier.align(Alignment.CenterHorizontally).padding(bottom = 10.dp), horizontalArrangement = Arrangement.spacedBy(8.dp), verticalAlignment = Alignment.CenterVertically) {
                    Spinner(c.inkMeta, size = 16.dp)
                    RText("Waiting", RType.sans(14f), c.inkMeta, maxLines = 1)
                }
            }
            SignalButton("Cancel", Modifier.fillMaxWidth().testTag("circleCancel"), style = ButtonStyle.Outline, onClick = onClose)
        }
        else -> {
            QrCode(invite.link, Modifier.align(Alignment.CenterHorizontally).testTag("circleQr"), size = 196.dp, label = "Invitation QR code")
            RText(
                invite.link,
                RType.mono(12.5f),
                c.ink2,
                Modifier.fillMaxWidth().padding(top = 12.dp).testTag("circleLink"),
                maxLines = 3,
                ltr = true,
            )
            Row(Modifier.fillMaxWidth().padding(top = 12.dp), horizontalArrangement = Arrangement.spacedBy(10.dp)) {
                SignalButton(
                    "Share",
                    Modifier.weight(1f).testTag("circleShare"),
                    leading = { Icon(Icons.Rounded.Share, null, Modifier.size(18.dp), tint = c.onInverse) },
                ) { share(context, invite.link) }
                SignalButton(
                    "Copy",
                    Modifier.weight(1f).testTag("circleCopy"),
                    style = ButtonStyle.Soft,
                    leading = { Icon(Icons.Rounded.ContentCopy, null, Modifier.size(18.dp), tint = c.ink) },
                ) { copy(context, invite.link) }
            }
            Row(Modifier.align(Alignment.CenterHorizontally).padding(vertical = 14.dp), horizontalArrangement = Arrangement.spacedBy(8.dp), verticalAlignment = Alignment.CenterVertically) {
                Spinner(c.inkMeta, size = 16.dp)
                RText("Waiting for them to open it", RType.sans(14f), c.inkMeta, Modifier.testTag("circleWaiting"), maxLines = 1)
            }
            SignalButton("Cancel", Modifier.fillMaxWidth().testTag("circleCancel"), style = ButtonStyle.Outline, onClick = onClose)
        }
    }
}

/** The six digits, big, in mono: what both phones show. */
@Composable
internal fun BigCode(code: String, modifier: Modifier = Modifier) {
    val c = LocalColors.current
    RText(
        formatCode(code),
        RType.mono(40f).copy(fontWeight = FontWeight.SemiBold, letterSpacing = 2.sp, fontFeatureSettings = "tnum"),
        c.ink,
        modifier.padding(vertical = 8.dp).testTag("circleCode"),
        maxLines = 1,
        ltr = true,
    )
}

/** A choice on its own row: an icon, a word, and a short line under it. */
@Composable
internal fun OptionRow(
    title: String,
    subtitle: String?,
    icon: ImageVector?,
    tag: String,
    chevron: Boolean = false,
    leading: (@Composable () -> Unit)? = null,
    onClick: () -> Unit,
) {
    val c = LocalColors.current
    val shape = RoundedCornerShape(16.dp)
    Row(
        Modifier
            .fillMaxWidth()
            .padding(vertical = 4.dp)
            .clip(shape)
            .background(c.paperSunken)
            .pressable(shape = shape, label = title, onClick = onClick)
            .heightIn(min = 56.dp)
            .padding(horizontal = 14.dp, vertical = 10.dp)
            .testTag(tag),
        horizontalArrangement = Arrangement.spacedBy(12.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        when {
            leading != null -> leading()
            icon != null -> Icon(icon, null, Modifier.size(22.dp), tint = c.ink2)
        }
        Column(Modifier.weight(1f)) {
            RText(title, RType.row, c.ink, maxLines = 2)
            subtitle?.let { RText(it, RType.meta, c.inkMeta, maxLines = 2) }
        }
        if (chevron) dev.reins.android.design.GlyphIcon(Glyph.ChevronRight, c.inkFaint, size = 18.dp)
    }
}

/** One of two choices side by side (Deferred · Instant). */
@Composable
private fun Segment(title: String, icon: ImageVector, selected: Boolean, tag: String, modifier: Modifier, onClick: () -> Unit) {
    val c = LocalColors.current
    val shape = RoundedCornerShape(14.dp)
    Row(
        modifier
            .clip(shape)
            .background(if (selected) c.violet.copy(alpha = 0.14f) else c.paperSunken)
            .then(if (selected) Modifier.border(1.5.dp, c.violet, shape) else Modifier)
            .pressable(shape = shape, label = title, onClick = onClick)
            .heightIn(min = 50.dp)
            .padding(horizontal = 12.dp)
            .testTag(tag),
        horizontalArrangement = Arrangement.spacedBy(8.dp, Alignment.CenterHorizontally),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Icon(icon, null, Modifier.size(18.dp), tint = if (selected) c.violet else c.ink2)
        RText(title, RType.sans(15f, FontWeight.SemiBold), if (selected) c.violet else c.ink, maxLines = 1)
    }
}

/**
 * The sheet both Circle flows use: from the bottom, the title in the serif, a back arrow inside the flow and a close
 * button. [onDismiss] is Back.
 */
@Composable
internal fun CircleSheet(
    title: String,
    tag: String,
    onBack: (() -> Unit)?,
    onDismiss: () -> Unit,
    onClose: () -> Unit,
    content: @Composable ColumnScope.() -> Unit,
) {
    val c = LocalColors.current
    Dialog(onDismissRequest = onDismiss, properties = DialogProperties(usePlatformDefaultWidth = false, dismissOnClickOutside = false)) {
        DialogFeedback()
        Box(Modifier.fillMaxSize(), contentAlignment = Alignment.BottomCenter) {
            val shape = RoundedCornerShape(topStart = 28.dp, topEnd = 28.dp)
            Column(
                Modifier
                    .fillMaxWidth()
                    .clip(shape)
                    .background(c.paperRaised)
                    .windowInsetsPadding(WindowInsets.navigationBars)
                    .imePadding()
                    .verticalScroll(rememberScrollState())
                    .padding(start = 22.dp, end = 22.dp, top = 16.dp, bottom = 16.dp)
                    .testTag(tag),
            ) {
                Row(Modifier.fillMaxWidth(), verticalAlignment = Alignment.CenterVertically) {
                    if (onBack != null) {
                        CircleIconButton(Glyph.ArrowBack, Modifier.testTag("circleSheetBack"), size = 36.dp, iconSize = 16.dp, label = "Back", onClick = onBack)
                        Spacer(Modifier.width(10.dp))
                    }
                    Box(Modifier.weight(1f)) {
                        AskText(title, c.ink, style = RType.serif(30f, 34f), minSize = 24.sp, maxLines = 2)
                    }
                    CircleIconButton(Glyph.Close, Modifier.testTag("circleSheetClose"), size = 36.dp, iconSize = 15.dp, label = "Close", onClick = onClose)
                }
                Spacer(Modifier.size(14.dp))
                content()
            }
        }
    }
}

/** The windows the core gives by default (2 min Instant, 4 h Deferred). */
private const val DEFERRED_SECS = 14_400L
private const val INSTANT_SECS = 120L

private fun share(context: Context, text: String) {
    val send = Intent(Intent.ACTION_SEND).setType("text/plain").putExtra(Intent.EXTRA_TEXT, text)
    context.startActivity(Intent.createChooser(send, "Share the invitation").addFlags(Intent.FLAG_ACTIVITY_NEW_TASK))
}

private fun copy(context: Context, text: String) {
    context.getSystemService(ClipboardManager::class.java)?.setPrimaryClip(ClipData.newPlainText("Reins Circle invitation", text))
}
