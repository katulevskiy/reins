package dev.reins.android.ui.org

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import dev.reins.android.design.Banner
import dev.reins.android.design.BannerKind
import dev.reins.android.design.ButtonSize
import dev.reins.android.design.ButtonStyle
import dev.reins.android.design.Card
import dev.reins.android.design.EmptyState
import dev.reins.android.design.Glyph
import dev.reins.android.design.LocalColors
import dev.reins.android.design.Pill
import dev.reins.android.design.RText
import dev.reins.android.design.RType
import dev.reins.android.design.Screen
import dev.reins.android.design.SignalButton
import dev.reins.android.ui.common.untrusted
import dev.reins.android.ui.common.untrustedName
import dev.reins.core.OrgReviewKind
import dev.reins.core.OrgReviewView

/**
 * Settings > Reviews (a work profile's): what waits for this member's review in their org. Each card shows the org, who
 * must approve, the title and the lines the phone built from what it signs; Approve and Deny sign with the key this
 * phone enrolled to the org. Your own admin change is confirmed instead.
 */
@Composable
fun OrgReviewsScreen(viewModel: OrgReviewsViewModel, onBack: () -> Unit) {
    val ui by viewModel.ui.collectAsStateWithLifecycle()
    LaunchedEffect(viewModel) { viewModel.load() }
    Screen(title = "Reviews", onBack = onBack) {
        Column(Modifier.fillMaxWidth().padding(horizontal = 16.dp, vertical = 10.dp), verticalArrangement = Arrangement.spacedBy(12.dp)) {
            ui.error?.let { Banner(it, kind = BannerKind.Error, tag = "orgReviewsError") }
            val reviews = ui.reviews
            when {
                reviews == null -> Unit
                reviews.isEmpty() -> EmptyState(Glyph.ShieldCheck, "No reviews", null, tag = "noOrgReviews")
                else -> reviews.forEach { review ->
                    ReviewCard(review, busy = ui.busy == review.id, enabled = ui.busy == null) { approve -> viewModel.answer(review, approve) }
                }
            }
        }
    }
}

@Composable
private fun ReviewCard(review: OrgReviewView, busy: Boolean, enabled: Boolean, onAnswer: (Boolean) -> Unit) {
    val c = LocalColors.current
    val confirm = review.own && review.kind == OrgReviewKind.CHANGE
    Card(Modifier.testTag("orgReview:${review.id}")) {
        Column(Modifier.padding(14.dp), verticalArrangement = Arrangement.spacedBy(8.dp)) {
            Row(horizontalArrangement = Arrangement.spacedBy(6.dp)) {
                Pill(untrustedName(review.orgName, 40), c.violet, glyph = Glyph.Briefcase)
                Pill("requires ${untrustedName(review.requires, 40)}", c.ink, glyph = Glyph.ShieldCheck)
            }
            RText(untrusted(review.title), RType.sans(16.5f, FontWeight.SemiBold), c.ink)
            val who = listOf(review.requester, review.connection).filter { it.isNotBlank() }.joinToString(" · ")
            if (who.isNotEmpty()) RText(untrusted(who), RType.sans(13.5f), c.inkMeta, maxLines = 1)
            review.lines.forEach { RText(untrusted(it), RType.sans(14.5f, lineHeight = 20f), c.ink) }
            Row(Modifier.fillMaxWidth().padding(top = 4.dp), horizontalArrangement = Arrangement.spacedBy(10.dp)) {
                SignalButton(
                    "Deny",
                    Modifier.weight(1f).testTag("orgDeny:${review.id}"),
                    style = ButtonStyle.DangerWash,
                    size = ButtonSize.Small,
                    glyph = Glyph.Close,
                    enabled = enabled,
                ) { onAnswer(false) }
                SignalButton(
                    if (confirm) "Confirm" else "Approve",
                    Modifier.weight(1f).testTag(if (confirm) "orgConfirm:${review.id}" else "orgApprove:${review.id}"),
                    size = ButtonSize.Small,
                    glyph = Glyph.ShieldCheck,
                    enabled = enabled && !busy,
                ) { onAnswer(true) }
            }
        }
    }
}
