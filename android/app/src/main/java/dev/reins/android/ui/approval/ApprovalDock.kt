package dev.reins.android.ui.approval

import androidx.compose.foundation.layout.Box
import androidx.compose.runtime.getValue
import androidx.lifecycle.compose.collectAsStateWithLifecycle

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
import androidx.compose.animation.animateColorAsState
import androidx.compose.ui.draw.shadow
import dev.reins.android.design.ButtonStyle
import dev.reins.android.design.ChooserSummary
import dev.reins.android.design.Glyph
import dev.reins.android.design.GlyphIcon
import dev.reins.android.design.Motion
import dev.reins.android.design.SignalButton

/**
 * The decision bar (Signal): "Allow for" (Once · 1 hour · 1 day · Custom ▾), locked to "Asked every time" when
 * nothing may be remembered, then Deny (outline) and the action (inverse, with the fingerprint). What can't be undone,
 * and paying, are held. It stays under every page of the screen.
 */
@Composable
internal fun Dock(view: ApprovalView, ui: ApprovalUi, viewModel: ApprovalViewModel, authenticator: Authenticator) {
    val c = LocalColors.current
    AskDock {
        ui.error?.let {
            RText(it, RType.sans(13.5f, FontWeight.Medium, lineHeight = 18f), c.danger, Modifier.fillMaxWidth().testTag("approvalError"), maxLines = 3)
        }
        when {
            view.kind == ApprovalKind.GRANT -> GrantPeriodRow(view, ui)
            // A red line on the field locks the chooser too: what it warns of is asked every time.
            rememberLocked(view, ui.draft) || view.purchase != null || heldByDanger(view) -> {
                LockedRow(if (mailPermissionSettings(view)) "Approve once" else "Asked every time")
                if (mailPermissionSettings(view)) RText("For ongoing access, use account or temporary permissions in Trust. Sensitive requests can still require review.", RType.sans(13f), c.ink2, Modifier.fillMaxWidth().testTag("accountPermissionHelp"))
            }
            else -> {
                if (likeThisOn(view, ui.draft)) LikeThisNote()
                envWiderLabel(view, ui.draft)?.let { EnvScopeNote(it) }
                Segments(view, ui, viewModel)
            }
        }
        // Later, Why? and a note with a denial (Decisions v2); a permission or a payment has none of them.
        if (view.kind != ApprovalKind.GRANT && view.purchase == null) {
            val item by viewModel.item.collectAsStateWithLifecycle()
            TicketActions(item, ui.busy, viewModel)
        }
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
            .clip(CircleShape)
            .background(c.warning.copy(alpha = 0.13f))
            .pressable(shape = CircleShape, label = "What it covers") { nav.overlay = ApprovalOverlay.Remember }
            .padding(horizontal = 12.dp, vertical = 6.dp)
            .testTag("likeThisNote"),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(6.dp),
    ) {
        GlyphIcon(Glyph.ChevronDown, c.warning, size = 14.dp)
        RText("Anything like this", RType.sans(12.5f, FontWeight.SemiBold), c.warning, maxLines = 1)
    }
}

/** A remembered release covers more than this exact command or these variables: the dock says what. */
@Composable
private fun EnvScopeNote(text: String) {
    val c = LocalColors.current
    val nav = LocalApprovalNav.current
    Row(
        Modifier
            .clip(CircleShape)
            .background(c.warning.copy(alpha = 0.13f))
            .pressable(shape = CircleShape, label = "What it covers") { nav.overlay = ApprovalOverlay.Remember }
            .padding(horizontal = 12.dp, vertical = 6.dp)
            .testTag("envScopeNote"),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(6.dp),
    ) {
        GlyphIcon(Glyph.ChevronDown, c.warning, size = 14.dp)
        RText(text, RType.mono(12.5f, FontWeight.SemiBold), c.warning, maxLines = 1, ltr = true)
    }
}

/** Nothing here can be remembered: the chooser's well, locked. */
@Composable
private fun LockedRow(text: String = "Asked every time") {
    val c = LocalColors.current
    Row(
        Modifier
            .fillMaxWidth()
            .height(46.dp)
            .clip(CircleShape)
            .background(c.paperSunken)
            .testTag("noStanding")
            .semantics(mergeDescendants = true) {},
        horizontalArrangement = Arrangement.spacedBy(8.dp, Alignment.CenterHorizontally),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        GlyphIcon(Glyph.Lock, c.ink2, size = 16.dp)
        RText(text, RType.sans(14f, FontWeight.Medium), c.ink2, maxLines = 1)
    }
}

/** A permission the AI asked for: "Allow for   1 hour · at most 5 uses ▾", which can only be made shorter. */
@Composable
private fun GrantPeriodRow(view: ApprovalView, ui: ApprovalUi) {
    val nav = LocalApprovalNav.current
    val feedback = LocalFeedback.current
    val asked = view.grant?.durationSecs?.toLong() ?: return
    val secs = ui.draft.grantSeconds?.coerceAtMost(asked) ?: asked
    val uses = view.grant?.maxUses?.let { " · at most $it uses" }.orEmpty()
    ChooserSummary("Allow for", "${periodWords(secs)}$uses", Modifier.testTag("grantPeriod")) {
        nav.overlay = ApprovalOverlay.Grant
        feedback.play(Event.Open)
    }
}

/** Once · 1 hour · 1 day · Custom ▾: the chooser's well with the chosen segment raised; Custom opens "Remember for". */
@Composable
private fun Segments(view: ApprovalView, ui: ApprovalUi, viewModel: ApprovalViewModel) {
    val c = LocalColors.current
    val nav = LocalApprovalNav.current
    val draft = ui.draft
    val on = segmentOf(draft)
    val accounts = view.kind == ApprovalKind.ACCOUNTS
    Row(Modifier.fillMaxWidth().clip(CircleShape).background(c.paperSunken).padding(4.dp), horizontalArrangement = Arrangement.spacedBy(2.dp)) {
        Seg("Once", on == Segment.Once, "lifetime:ONCE") { viewModel.edit { it.copy(lifetime = LifetimeKind.ONCE, allMail = null) } }
        Seg(if (accounts) "1 hour" else shortLabel(view), on == Segment.Short, "lifetime:HOUR") {
            viewModel.edit { it.copy(lifetime = LifetimeKind.HOUR, allMail = null) }
        }
        Seg("1 day", on == Segment.Day, "lifetime:DAY") { viewModel.edit { it.copy(lifetime = LifetimeKind.DAY, allMail = null) } }
        Seg(customLabel(draft), on == Segment.Custom, "lifetimeCustom", menu = true) {
            nav.overlay = ApprovalOverlay.Remember
        }
    }
}

@Composable
private fun RowScope.Seg(title: String, selected: Boolean, tag: String, menu: Boolean = false, onClick: () -> Unit) {
    val c = LocalColors.current
    val feedback = LocalFeedback.current
    val bg by animateColorAsState(if (selected) c.paperRaised else c.paperSunken.copy(alpha = 0f), Motion.fade(), label = "segment")
    Row(
        Modifier
            .weight(1f)
            .height(38.dp)
            .then(if (selected) Modifier.shadow(1.dp, CircleShape, ambientColor = Color.Black.copy(alpha = 0.08f), spotColor = Color.Black.copy(alpha = 0.08f)) else Modifier)
            .clip(CircleShape)
            .background(bg)
            .then(if (selected) Modifier.border(1.dp, c.rule, CircleShape) else Modifier)
            .pressable(shape = CircleShape) {
                onClick()
                feedback.play(if (menu) Event.Open else Event.Selection)
            }
            .semantics {
                role = Role.Tab
                this.selected = selected
            }
            .testTag(tag)
            .padding(horizontal = 4.dp),
        horizontalArrangement = Arrangement.spacedBy(3.dp, Alignment.CenterHorizontally),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        RText(title, RType.sans(14f, FontWeight.Medium), if (selected) c.ink else c.ink2, Modifier.weight(1f, fill = false), maxLines = 1)
        if (menu) GlyphIcon(Glyph.ChevronDown, if (selected) c.ink else c.ink2, size = 14.dp)
    }
}

/** Deny (outline) and the action (inverse, with the fingerprint); what can't be undone, and paying, are held in red. */
@Composable
private fun Buttons(view: ApprovalView, ui: ApprovalUi, viewModel: ApprovalViewModel, authenticator: Authenticator) {
    val c = LocalColors.current
    Row(Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.spacedBy(10.dp)) {
        SignalButton(denyLabel(view), Modifier.weight(1f).testTag("deny"), style = ButtonStyle.Outline, enabled = !ui.busy, onClick = viewModel::deny)
        val purchase = view.purchase
        val approveModifier = Modifier.weight(1.35f)
        when {
            purchase != null -> {
                val context = LocalContext.current
                val onPhone = purchase.methods.firstOrNull { it.id == ui.purchase.methodId }?.kind == "pay_on_phone"
                val pay = { viewModel.approvePurchase(authenticator) { url -> dev.reins.android.platform.Browser.open(context, url) } }
                if (onPhone) {
                    ActionButton("Open checkout", approveModifier.testTag("approve"), ui.busy, onClick = pay)
                } else {
                    // Paying is held, like a delete: money can't come back.
                    HoldButton("Hold to pay", c.danger, approveModifier.testTag("approve"), icon = Icons.Rounded.Fingerprint, enabled = !ui.busy, busy = ui.busy, onConfirm = pay)
                }
            }
            heldByDanger(view) -> HoldButton(
                "Hold to ${approveLabel(view, ui.draft).lowercase()}",
                c.danger,
                approveModifier.testTag("approve"),
                icon = Icons.Rounded.Fingerprint,
                enabled = !ui.busy,
                busy = ui.busy,
            ) { viewModel.approve(authenticator) }
            else -> ActionButton(approveLabel(view, ui.draft), approveModifier.testTag("approve"), ui.busy) { viewModel.approve(authenticator) }
        }
    }
}

/** The action: inverse, with the fingerprint glyph (BiometricPrompt "Approve request" follows the tap). */
@Composable
private fun ActionButton(title: String, modifier: Modifier, busy: Boolean, onClick: () -> Unit) {
    val c = LocalColors.current
    SignalButton(
        title,
        modifier,
        enabled = !busy,
        leading = { if (busy) Spinner(c.onInverse, size = 18.dp) else GlyphIcon(Glyph.Fingerprint, c.onInverse, size = 20.dp) },
        onClick = onClick,
    )
}
