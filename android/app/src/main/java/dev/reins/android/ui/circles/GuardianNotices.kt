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
import androidx.compose.material.icons.rounded.Check
import androidx.compose.material.icons.rounded.FamilyRestroom
import androidx.compose.material.icons.rounded.Info
import androidx.compose.material.icons.rounded.Verified
import androidx.compose.material3.Icon
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import dev.reins.android.design.Banner
import dev.reins.android.design.BannerKind
import dev.reins.android.design.ButtonSize
import dev.reins.android.design.ButtonStyle
import dev.reins.android.design.Card
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
import dev.reins.android.design.Spinner
import dev.reins.android.ui.common.relativeTime
import dev.reins.android.ui.common.untrusted
import dev.reins.core.GuardianNoticeView

/**
 * The guardian's side of a guardianship (Decisions v2 P7, 04 §4): what this phone was told about the ward, newest first.
 * What it checked itself (a signed change, the roster chain) shows plainly; what only the server says is labelled
 * "Reported by the server". A minor's request to leave waits here for Allow. The guardian may ask to mark the ward's
 * account a minor's; the ward accepts on their phone, or not.
 */
@Composable
fun GuardianNoticesScreen(
    guarding: GuardingUi,
    onAllow: (GuardianNoticeView) -> Unit,
    onProposeMinor: () -> Unit,
    onBack: () -> Unit,
) {
    val c = LocalColors.current
    BackHandler(onBack = onBack)
    val ward = untrusted(guarding.person.peer)
    Screen(title = shortName(ward), subtitle = personLine(guarding.person), onBack = onBack, modifier = Modifier.testTag("guardianNotices")) {
        guarding.error?.let { Banner(it, Modifier.padding(horizontal = 16.dp, vertical = 6.dp), BannerKind.Error, tag = "guardianError") }
        val notices = guarding.notices
        if (notices == null) {
            RText("Loading…", RType.sans(15f), c.ink2, Modifier.padding(horizontal = 20.dp, vertical = 12.dp))
            return@Screen
        }
        notices.filter { it.needsConsent }.forEach { notice ->
            ConsentCard(notice, busy = guarding.busy == notice.id, enabled = guarding.busy == null) { onAllow(notice) }
        }
        Group(header = "Notices", footer = "What changed on ${shortName(ward)}'s side. Checked on this phone, unless it says Reported by the server.") {
            if (notices.isEmpty()) {
                RText("Nothing yet.", RType.sans(15f), c.ink2, Modifier.padding(vertical = 14.dp).testTag("guardianNoNotices"))
            }
            notices.forEachIndexed { i, notice ->
                if (i > 0) Hairline(inset = 30.dp)
                NoticeRow(notice)
            }
        }
        MinorAsk(ward, guarding, onProposeMinor)
        Spacer(Modifier.padding(bottom = 32.dp))
    }
}

/** "Sam asked to leave. Allow?": a minor's leaving ends only with the guardian's consent. */
@Composable
private fun ConsentCard(notice: GuardianNoticeView, busy: Boolean, enabled: Boolean, onAllow: () -> Unit) {
    val c = LocalColors.current
    Card(Modifier.padding(horizontal = 16.dp, vertical = 8.dp).testTag("guardianConsent:${notice.id}")) {
        RText(consentLine(untrusted(notice.ward)), RType.sans(16f, FontWeight.SemiBold), c.ink, maxLines = 2)
        if (!notice.verified) Unverified(notice.id)
        SignalButton(
            "Allow",
            Modifier.fillMaxWidth().padding(top = 12.dp).testTag("guardianAllow:${notice.id}"),
            size = ButtonSize.Small,
            enabled = enabled,
            leading = { if (busy) Spinner(c.onInverse, size = 18.dp) else Icon(Icons.Rounded.Check, null, Modifier.size(18.dp), tint = c.onInverse) },
            onClick = onAllow,
        )
    }
}

@Composable
private fun NoticeRow(notice: GuardianNoticeView) {
    val c = LocalColors.current
    Row(
        Modifier.fillMaxWidth().padding(vertical = 10.dp).testTag("guardianNotice:${notice.id}"),
        horizontalArrangement = Arrangement.spacedBy(12.dp),
        verticalAlignment = Alignment.Top,
    ) {
        Icon(
            if (notice.verified) Icons.Rounded.Verified else Icons.Rounded.Info,
            null,
            Modifier.padding(top = 2.dp).size(18.dp),
            tint = if (notice.verified) c.ink2 else c.warning,
        )
        Column(Modifier.weight(1f), verticalArrangement = Arrangement.spacedBy(2.dp)) {
            RText(untrusted(notice.line), RType.row, c.ink, Modifier.testTag("guardianNoticeLine:${notice.id}"), maxLines = 2)
            RText(relativeTime(notice.at), RType.meta, c.inkMeta, maxLines = 1)
            if (!notice.verified) Unverified(notice.id)
        }
    }
}

/** What this phone could not check. */
@Composable
private fun Unverified(id: String) {
    NextPill(REPORTED_BY_SERVER, LocalColors.current.warning, Modifier.padding(top = 4.dp), textTag = "guardianUnverified:$id")
}

/** Ask to mark the ward's account a minor's (then leaving needs this account's consent). */
@Composable
private fun MinorAsk(ward: String, guarding: GuardingUi, onPropose: () -> Unit) {
    val c = LocalColors.current
    var help by rememberSaveable { mutableStateOf(false) }
    Column(Modifier.padding(horizontal = 20.dp, vertical = 18.dp)) {
        Row(Modifier.fillMaxWidth(), verticalAlignment = Alignment.CenterVertically) {
            if (guarding.minorAsked) {
                Icon(Icons.Rounded.FamilyRestroom, null, Modifier.size(18.dp), tint = c.inkMeta)
                RText("Asked ${shortName(ward)}", RType.sans(15f), c.inkMeta, Modifier.weight(1f).padding(start = 10.dp).testTag("guardianMinorAsked"), maxLines = 1)
            } else {
                SignalButton(
                    "Minor's account",
                    Modifier.weight(1f).testTag("guardianProposeMinor"),
                    style = ButtonStyle.Soft,
                    size = ButtonSize.Small,
                    enabled = guarding.busy == null,
                    leading = {
                        if (guarding.busy == CirclesViewModel.MINOR) Spinner(c.ink, size = 18.dp) else Icon(Icons.Rounded.FamilyRestroom, null, Modifier.size(18.dp), tint = c.ink)
                    },
                    onClick = onPropose,
                )
            }
            HelpButton(help, "Minor's account") { help = it }
        }
        HelpText(
            "${shortName(ward)} accepts on their phone, or not. Then leaving needs your OK.",
            help,
            Modifier.padding(top = 6.dp),
        )
    }
}
