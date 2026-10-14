package dev.reins.android.ui.approval

import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.RowScope
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.WindowInsets
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.navigationBars
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.layout.windowInsetsPadding
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.rounded.Fingerprint
import androidx.compose.material.icons.rounded.KeyboardArrowDown
import androidx.compose.material.icons.rounded.Lock
import androidx.compose.material.icons.rounded.UnfoldMore
import androidx.compose.material3.Icon
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.graphicsLayer
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.role
import androidx.compose.ui.semantics.selected
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import dev.reins.android.design.HoldButton
import dev.reins.android.design.LocalColors
import dev.reins.android.design.RText
import dev.reins.android.design.RType
import dev.reins.android.design.Spinner
import dev.reins.android.design.pressable
import dev.reins.android.feedback.Event
import dev.reins.android.feedback.LocalFeedback
import dev.reins.android.feedback.play
import dev.reins.android.platform.Authenticator
import dev.reins.core.ApprovalKind
import dev.reins.core.ApprovalView

/**
 * The dock: how long to remember it (Once · 1 hour · 1 day · Custom ▾, or "Asked every time" when nothing may be
 * remembered), then Deny and Approve. It stays under every page of the screen.
 */
@Composable
internal fun Dock(view: ApprovalView, ui: ApprovalUi, viewModel: ApprovalViewModel, authenticator: Authenticator) {
    val c = LocalColors.current
    val shape = RoundedCornerShape(topStart = 28.dp, topEnd = 28.dp)
    Column(
        Modifier
            .fillMaxWidth()
            .clip(shape)
            .background(Surfaces.dock(c))
            .border(1.dp, Surfaces.dockEdge(c), shape)
            .windowInsetsPadding(WindowInsets.navigationBars)
            .padding(start = 14.dp, end = 14.dp, top = 12.dp, bottom = 14.dp)
            .testTag("dock"),
    ) {
        ui.error?.let {
            RText(it, RType.sans(13.5f, FontWeight.Medium, lineHeight = 18f), c.danger, Modifier.fillMaxWidth().padding(start = 6.dp, end = 6.dp, bottom = 10.dp).testTag("approvalError"), maxLines = 3)
        }
        when {
            view.kind == ApprovalKind.GRANT -> GrantPeriodRow(view, ui)
            rememberLocked(view) -> LockedRow()
            else -> {
                if (likeThisOn(view, ui.draft)) LikeThisNote()
                Segments(view, ui, viewModel)
            }
        }
        Spacer(Modifier.height(10.dp))
        Buttons(view, ui, viewModel, authenticator)
    }
}

/** "Anything like this" was chosen: the dock says the remembered approval is wider than the screen. */
@Composable
private fun LikeThisNote() {
    val c = LocalColors.current
    val nav = LocalApprovalNav.current
    Row(
        Modifier
            .padding(bottom = 8.dp)
            .clip(CircleShape)
            .background(c.warning.copy(alpha = 0.13f))
            .pressable(shape = CircleShape, label = "What it covers") { nav.overlay = ApprovalOverlay.Remember }
            .padding(horizontal = 12.dp, vertical = 6.dp)
            .testTag("likeThisNote"),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Icon(Icons.Rounded.UnfoldMore, null, Modifier.size(15.dp), tint = c.warning)
        Spacer(Modifier.width(6.dp))
        RText("Anything like this", RType.sans(12.5f, FontWeight.SemiBold), c.warning, maxLines = 1)
    }
}

/** Nothing here can be remembered. */
@Composable
private fun LockedRow() {
    val c = LocalColors.current
    Row(
        Modifier
            .fillMaxWidth()
            .clip(RoundedCornerShape(14.dp))
            .background(Surfaces.sunken(c))
            .padding(11.dp)
            .testTag("noStanding")
            .semantics(mergeDescendants = true) {},
        horizontalArrangement = Arrangement.Center,
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Icon(Icons.Rounded.Lock, null, Modifier.size(15.dp), tint = c.secondary)
        Spacer(Modifier.width(8.dp))
        RText("Asked every time", RType.sans(13f, FontWeight.Medium), c.secondary, maxLines = 1)
    }
}

/** A permission the AI asked for: how long, which can only be made shorter in its sheet. */
@Composable
private fun GrantPeriodRow(view: ApprovalView, ui: ApprovalUi) {
    val c = LocalColors.current
    val nav = LocalApprovalNav.current
    val feedback = LocalFeedback.current
    val asked = view.grant?.durationSecs?.toLong() ?: return
    val secs = ui.draft.grantSeconds?.coerceAtMost(asked) ?: asked
    Row(
        Modifier
            .fillMaxWidth()
            .clip(RoundedCornerShape(14.dp))
            .background(Surfaces.sunken(c))
            .pressable(shape = RoundedCornerShape(14.dp), label = "How long") {
                nav.overlay = ApprovalOverlay.Grant
                feedback.play(Event.Open)
            }
            .padding(11.dp)
            .testTag("grantPeriod"),
        horizontalArrangement = Arrangement.Center,
        verticalAlignment = Alignment.CenterVertically,
    ) {
        RText("For ${periodWords(secs)}", RType.sans(13.5f, FontWeight.SemiBold), c.text, maxLines = 1)
        Spacer(Modifier.width(3.dp))
        Icon(Icons.Rounded.KeyboardArrowDown, null, Modifier.size(16.dp), tint = c.secondary)
    }
}

/** Once · 1 hour · 1 day · Custom ▾ as a connected button group (Material 3 Expressive). */
@Composable
private fun Segments(view: ApprovalView, ui: ApprovalUi, viewModel: ApprovalViewModel) {
    val nav = LocalApprovalNav.current
    val draft = ui.draft
    val on = segmentOf(draft)
    val accounts = view.kind == ApprovalKind.ACCOUNTS
    Row(Modifier.fillMaxWidth().height(40.dp), horizontalArrangement = Arrangement.spacedBy(3.dp)) {
        Seg("Once", on == Segment.Once, 0, "lifetime:ONCE") { viewModel.edit { it.copy(lifetime = LifetimeKind.ONCE, allMail = null) } }
        Seg(if (accounts) "1 hour" else shortLabel(view), on == Segment.Short, 1, "lifetime:HOUR") {
            viewModel.edit { it.copy(lifetime = LifetimeKind.HOUR, allMail = null) }
        }
        Seg("1 day", on == Segment.Day, 2, "lifetime:DAY") { viewModel.edit { it.copy(lifetime = LifetimeKind.DAY, allMail = null) } }
        Seg(customLabel(draft), on == Segment.Custom, 3, "lifetimeCustom", menu = true) {
            nav.overlay = ApprovalOverlay.Remember
        }
    }
}

@Composable
private fun RowScope.Seg(title: String, selected: Boolean, index: Int, tag: String, menu: Boolean = false, onClick: () -> Unit) {
    val c = LocalColors.current
    val feedback = LocalFeedback.current
    val outer = 20.dp
    val inner = 6.dp
    val shape = when {
        selected -> RoundedCornerShape(outer)
        index == 0 -> RoundedCornerShape(topStart = outer, bottomStart = outer, topEnd = inner, bottomEnd = inner)
        index == 3 -> RoundedCornerShape(topStart = inner, bottomStart = inner, topEnd = outer, bottomEnd = outer)
        else -> RoundedCornerShape(inner)
    }
    val fg = if (selected) (if (c.dark) Color(0xFFDCD6FF) else c.accent) else c.secondary
    Row(
        Modifier
            .weight(1f)
            .height(40.dp)
            .clip(shape)
            .background(if (selected) c.accent.copy(alpha = 0.24f) else Surfaces.segment(c))
            .pressable(shape = shape) {
                onClick()
                feedback.play(if (menu) Event.Open else Event.Selection)
            }
            .semantics {
                role = Role.Tab
                this.selected = selected
            }
            .testTag(tag),
        horizontalArrangement = Arrangement.Center,
        verticalAlignment = Alignment.CenterVertically,
    ) {
        RText(title, RType.sans(13.5f, FontWeight.SemiBold), fg, Modifier.weight(1f, fill = false), maxLines = 1)
        if (menu) {
            Spacer(Modifier.width(2.dp))
            Icon(Icons.Rounded.KeyboardArrowDown, null, Modifier.size(15.dp), tint = fg)
        }
    }
}

/** Deny (quiet) and Approve (light, with the fingerprint); a destructive Approve is red and held. */
@Composable
private fun Buttons(view: ApprovalView, ui: ApprovalUi, viewModel: ApprovalViewModel, authenticator: Authenticator) {
    val c = LocalColors.current
    Row(Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.spacedBy(10.dp)) {
        DockButton(denyLabel(view), Modifier.weight(1f).testTag("deny"), Surfaces.raised(c), c.text, enabled = !ui.busy, onClick = viewModel::deny)
        val purchase = view.purchase
        val approveModifier = Modifier.weight(1.35f)
        when {
            purchase != null -> {
                val context = LocalContext.current
                val onPhone = purchase.methods.firstOrNull { it.id == ui.purchase.methodId }?.kind == "pay_on_phone"
                DockButton(
                    if (onPhone) "Open checkout" else "Pay ${purchase.total}",
                    approveModifier.testTag("approve"),
                    c.text,
                    c.background,
                    enabled = !ui.busy,
                    busy = ui.busy,
                    fingerprint = true,
                ) { viewModel.approvePurchase(authenticator) { url -> dev.reins.android.platform.Browser.open(context, url) } }
            }
            destructive(view) -> HoldButton(
                "Hold to ${approveLabel(view, ui.draft).lowercase()}",
                c.danger,
                approveModifier.testTag("approve"),
                icon = Icons.Rounded.Fingerprint,
                enabled = !ui.busy,
                busy = ui.busy,
            ) { viewModel.approve(authenticator) }
            else -> DockButton(
                approveLabel(view, ui.draft),
                approveModifier.testTag("approve"),
                c.text,
                c.background,
                enabled = !ui.busy,
                busy = ui.busy,
                fingerprint = true,
            ) { viewModel.approve(authenticator) }
        }
    }
}

@Composable
private fun DockButton(
    title: String,
    modifier: Modifier,
    bg: Color,
    fg: Color,
    enabled: Boolean = true,
    busy: Boolean = false,
    fingerprint: Boolean = false,
    onClick: () -> Unit,
) {
    Row(
        modifier
            .height(56.dp)
            .graphicsLayer { alpha = if (enabled || busy) 1f else 0.45f }
            .clip(CircleShape)
            .background(bg)
            .pressable(enabled = enabled && !busy, shape = CircleShape, onClick = onClick)
            .padding(horizontal = 16.dp),
        horizontalArrangement = Arrangement.Center,
        verticalAlignment = Alignment.CenterVertically,
    ) {
        if (busy) {
            Spinner(fg, size = 18.dp)
            Spacer(Modifier.width(8.dp))
        } else if (fingerprint) {
            Icon(Icons.Rounded.Fingerprint, null, Modifier.size(20.dp), tint = fg.copy(alpha = 0.8f))
            Spacer(Modifier.width(8.dp))
        }
        RText(title, RType.sans(16f, FontWeight.SemiBold), fg, Modifier.weight(1f, fill = false), maxLines = 1)
    }
}
