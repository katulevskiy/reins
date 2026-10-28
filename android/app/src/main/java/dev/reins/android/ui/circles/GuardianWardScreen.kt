package dev.reins.android.ui.circles

import androidx.activity.compose.BackHandler
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.rounded.Block
import androidx.compose.material.icons.rounded.Check
import androidx.compose.material.icons.rounded.Close
import androidx.compose.material.icons.rounded.FamilyRestroom
import androidx.compose.material.icons.rounded.Handshake
import androidx.compose.material.icons.automirrored.rounded.Logout
import androidx.compose.material.icons.rounded.Schedule
import androidx.compose.material3.Icon
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import dev.reins.android.design.Banner
import dev.reins.android.design.BannerKind
import dev.reins.android.design.ButtonSize
import dev.reins.android.design.ButtonStyle
import dev.reins.android.design.Card
import dev.reins.android.design.Glyph
import dev.reins.android.design.Group
import dev.reins.android.design.Hairline
import dev.reins.android.design.HelpButton
import dev.reins.android.design.HelpText
import dev.reins.android.design.LocalColors
import dev.reins.android.design.NextPill
import dev.reins.android.design.RText
import dev.reins.android.design.RType
import dev.reins.android.design.Screen
import dev.reins.android.design.SignalButton
import dev.reins.android.design.SignalDialog
import dev.reins.android.design.Spinner
import dev.reins.android.design.SwitchRow
import dev.reins.android.design.rememberNowState
import dev.reins.android.ui.common.relativeTime
import dev.reins.android.ui.common.untrusted
import dev.reins.core.GuardianDecisionView
import dev.reins.core.WardView

/**
 * The ward's own guardianship (Decisions v2 P7, 04 §4): what it covers and how it ends, the guardian's minor ask, the
 * weekly summary (off until the ward turns it on), every decision the guardian made with its note, and Leave. It is
 * cooperative: leaving is always possible, after the notice the ward chose (a minor's waits for the guardian).
 */
@Composable
fun GuardianWardScreen(
    ward: WardUi,
    onLeave: () -> Unit,
    onAcceptMinor: () -> Unit,
    onIgnoreMinor: () -> Unit,
    onDigest: (Boolean) -> Unit,
    onBack: () -> Unit,
) {
    val c = LocalColors.current
    BackHandler(onBack = onBack)
    val guardian = untrusted(ward.view?.guardian ?: ward.person.peer)
    var leaving by rememberSaveable { mutableStateOf(false) }
    Screen(title = shortName(guardian), subtitle = "Guardian", onBack = onBack, modifier = Modifier.testTag("guardianWard")) {
        ward.error?.let { Banner(it, Modifier.padding(horizontal = 16.dp, vertical = 6.dp), BannerKind.Error, tag = "guardianError") }
        val view = ward.view
        if (view == null) {
            RText("Loading…", RType.sans(15f), c.ink2, Modifier.padding(horizontal = 20.dp, vertical = 12.dp))
            return@Screen
        }
        if (view.minorProposed && !view.minor && !ward.minorIgnored) {
            MinorCard(guardian, ward.busy, onAcceptMinor, onIgnoreMinor)
        }
        Terms(view, guardian)
        Leave(view, guardian, ward.busy) { leaving = true }
        Group(header = "Summary", footer = "Once a week: how many automatic decisions in scope, and what kind. No messages or content.") {
            SwitchRow(digestLabel(guardian), view.digest, onDigest, enabled = !ward.busy, tag = "guardianDigest")
        }
        Decisions(view.decisions)
        Spacer(Modifier.padding(bottom = 32.dp))
    }
    if (leaving) {
        val days = ward.view?.noticeDays?.toInt() ?: 0
        SignalDialog(
            title = "Leave ${shortName(guardian)}'s guardianship?",
            text = if (ward.view?.minor == true) "${shortName(guardian)} must allow it." else "${noticeLine(days)}. ${shortName(guardian)} is told.",
            confirmLabel = "Leave",
            onConfirm = {
                leaving = false
                onLeave()
            },
            onDismiss = { leaving = false },
            destructive = true,
            confirmTag = "guardianLeaveConfirm",
            glyph = Glyph.People,
        )
    }
}

/** What it covers, the notice, and whether this is marked a minor's account. */
@Composable
private fun Terms(view: WardView, guardian: String) {
    Group(header = "Guardianship", footer = "${shortName(guardian)} approves what this covers, and sees only those requests. It relies on both phones: leaving early is shown to ${shortName(guardian)}.") {
        untrusted(view.scopeLine).ifBlank { null }?.let { FactRow(Icons.Rounded.Handshake, it, "guardianScope") }
        Hairline(inset = 30.dp)
        FactRow(Icons.Rounded.Schedule, noticeLine(view.noticeDays.toInt()), "guardianNotice")
        if (view.minor) {
            Hairline(inset = 30.dp)
            FactRow(Icons.Rounded.FamilyRestroom, "Minor's account", "guardianMinor")
        }
    }
}

/** Leave, or how the leaving stands: the countdown, or who it waits for. */
@Composable
private fun Leave(view: WardView, guardian: String, busy: Boolean, onLeave: () -> Unit) {
    val c = LocalColors.current
    val ending = view.ending
    Group(header = "Leave") {
        when {
            view.needsConsent -> Row(Modifier.fillMaxWidth().padding(vertical = 12.dp), horizontalArrangement = Arrangement.spacedBy(10.dp), verticalAlignment = Alignment.CenterVertically) {
                Spinner(c.inkMeta, size = 16.dp)
                RText(waitingFor(guardian), RType.row, c.ink, Modifier.testTag("guardianWaiting"), maxLines = 1)
            }
            ending != null -> {
                val now by rememberNowState(30_000)
                Row(Modifier.fillMaxWidth().padding(vertical = 12.dp), horizontalArrangement = Arrangement.spacedBy(10.dp), verticalAlignment = Alignment.CenterVertically) {
                    Icon(Icons.Rounded.Schedule, null, Modifier.size(18.dp), tint = c.warning)
                    RText(endingLine(ending, now / 1000), RType.row, c.ink, Modifier.testTag("guardianEnding"), maxLines = 1)
                }
            }
            else -> Row(Modifier.fillMaxWidth().padding(vertical = 10.dp), verticalAlignment = Alignment.CenterVertically) {
                RText(noticeLine(view.noticeDays.toInt()), RType.meta, c.inkMeta, Modifier.weight(1f), maxLines = 2)
                SignalButton(
                    "Leave",
                    Modifier.testTag("guardianLeave"),
                    style = ButtonStyle.Outline,
                    size = ButtonSize.Small,
                    enabled = !busy,
                    leading = { Icon(Icons.AutoMirrored.Rounded.Logout, null, Modifier.size(18.dp), tint = c.danger) },
                    onClick = onLeave,
                )
            }
        }
    }
}

/** The guardian asked to mark this a minor's account: Accept (with the phone's age signal) or Ignore. */
@Composable
private fun MinorCard(guardian: String, busy: Boolean, onAccept: () -> Unit, onIgnore: () -> Unit) {
    val c = LocalColors.current
    var help by rememberSaveable { mutableStateOf(false) }
    Card(Modifier.padding(horizontal = 16.dp, vertical = 8.dp).testTag("guardianMinorCard")) {
        Row(Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.spacedBy(10.dp), verticalAlignment = Alignment.CenterVertically) {
            Icon(Icons.Rounded.FamilyRestroom, null, Modifier.size(22.dp), tint = c.ink2)
            RText("Minor's account?", RType.sans(16f, FontWeight.SemiBold), c.ink, Modifier.weight(1f), maxLines = 1)
            HelpButton(help, "Minor's account") { help = it }
        }
        RText("Asked by ${shortName(guardian)}", RType.meta, c.inkMeta, Modifier.padding(top = 2.dp), maxLines = 1)
        HelpText(
            "Then leaving needs ${shortName(guardian)}'s OK. Google Play's age check is used where it's available.",
            help,
            Modifier.padding(top = 6.dp),
        )
        Row(Modifier.fillMaxWidth().padding(top = 12.dp), horizontalArrangement = Arrangement.spacedBy(10.dp)) {
            SignalButton(
                "Ignore",
                Modifier.weight(1f).testTag("guardianMinorIgnore"),
                style = ButtonStyle.Outline,
                size = ButtonSize.Small,
                enabled = !busy,
                leading = { Icon(Icons.Rounded.Close, null, Modifier.size(18.dp), tint = c.ink) },
                onClick = onIgnore,
            )
            SignalButton(
                "Accept",
                Modifier.weight(1f).testTag("guardianMinorAccept"),
                size = ButtonSize.Small,
                enabled = !busy,
                leading = { if (busy) Spinner(c.onInverse, size = 18.dp) else Icon(Icons.Rounded.Check, null, Modifier.size(18.dp), tint = c.onInverse) },
                onClick = onAccept,
            )
        }
    }
}

/** Every decision the guardian made, newest first, with its note. */
@Composable
private fun Decisions(decisions: List<GuardianDecisionView>) {
    val c = LocalColors.current
    Group(header = "Decisions") {
        if (decisions.isEmpty()) {
            RText("None yet.", RType.sans(15f), c.ink2, Modifier.padding(vertical = 14.dp).testTag("guardianNoDecisions"))
        }
        decisions.forEachIndexed { i, d ->
            if (i > 0) Hairline(inset = 30.dp)
            DecisionRow(d, i)
        }
    }
}

@Composable
private fun DecisionRow(d: GuardianDecisionView, index: Int) {
    val c = LocalColors.current
    Row(
        Modifier.fillMaxWidth().padding(vertical = 10.dp).testTag("guardianDecision:$index"),
        horizontalArrangement = Arrangement.spacedBy(12.dp),
        verticalAlignment = Alignment.Top,
    ) {
        Icon(
            if (d.approved) Icons.Rounded.Check else Icons.Rounded.Block,
            null,
            Modifier.padding(top = 2.dp).size(18.dp),
            tint = if (d.approved) c.success else c.danger,
        )
        Column(Modifier.weight(1f), verticalArrangement = Arrangement.spacedBy(2.dp)) {
            RText(untrusted(d.title), RType.row, c.ink, maxLines = 2)
            RText("${if (d.approved) "Approved" else "Denied"} · ${relativeTime(d.at)}", RType.meta, c.inkMeta, maxLines = 1)
            d.note?.let(::untrusted)?.ifBlank { null }?.let {
                RText(it, RType.sans(14f), c.ink2, Modifier.padding(top = 2.dp).testTag("guardianNote:$index"), maxLines = 4)
            }
        }
    }
}

@Composable
internal fun FactRow(icon: ImageVector, text: String, tag: String) {
    val c = LocalColors.current
    Row(Modifier.fillMaxWidth().padding(vertical = 12.dp), horizontalArrangement = Arrangement.spacedBy(12.dp), verticalAlignment = Alignment.CenterVertically) {
        Icon(icon, null, Modifier.size(18.dp), tint = c.ink2)
        RText(text, RType.row, c.ink, Modifier.weight(1f).testTag(tag), maxLines = 2)
    }
}

/** A badge for a guardianship that is ending, on the ward's or the guardian's row. */
@Composable
internal fun EndingPill(text: String, tag: String) {
    NextPill(text, LocalColors.current.warning, Modifier.padding(top = 4.dp), textTag = tag)
}
