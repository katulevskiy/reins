package dev.reins.android.ui.activity

import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.offset
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.rounded.Block
import androidx.compose.material.icons.rounded.Computer
import androidx.compose.material.icons.rounded.Fingerprint
import androidx.compose.material.icons.rounded.Lock
import androidx.compose.material.icons.rounded.MailOutline
import androidx.compose.material.icons.rounded.PhoneAndroid
import androidx.compose.material.icons.rounded.UploadFile
import androidx.compose.material3.Icon
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.State
import androidx.compose.runtime.derivedStateOf
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateMapOf
import androidx.compose.runtime.remember
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.SpanStyle
import androidx.compose.ui.text.buildAnnotatedString
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.withStyle
import androidx.compose.ui.unit.dp
import dev.reins.android.AppContainer
import dev.reins.android.design.LocalColors
import dev.reins.android.design.MiniLogo
import dev.reins.android.design.ProgressRing
import dev.reins.android.design.RColors
import dev.reins.android.design.RText
import dev.reins.android.design.RType
import dev.reins.android.design.onSuccess
import dev.reins.android.design.pressable
import dev.reins.android.design.rememberNowState
import dev.reins.android.design.toneCard
import dev.reins.android.design.urgency
import dev.reins.android.ui.common.ConnectionIcon
import dev.reins.android.ui.settings.PillButton
import dev.reins.android.ui.settings.primaryFill
import dev.reins.core.ApprovalView
import dev.reins.core.PendingItem
import dev.reins.core.PendingKind
import dev.reins.core.SuggestionView
import kotlin.coroutines.cancellation.CancellationException

/**
 * What the waiting cards know beyond the list: Autopilot's suggestion and the request read ahead, per request id.
 * Read once per request, in the background; a card draws without them until they come.
 */
class PendingDetails {
    val suggestions = mutableStateMapOf<String, SuggestionView?>()
    val views = mutableStateMapOf<String, ApprovalView>()

    /** Autopilot's reason line per request ("Like 11 pushes to feature branches of acme/web you approved"). */
    val reasons = mutableStateMapOf<String, String>()

    /** The push each decided entry made (repo and branch kind), from its stored approval; null when it was none. */
    internal val pushes = HashMap<Long, PushKey?>()
}

@Composable
fun rememberPendingDetails(container: AppContainer, pending: List<PendingItem>, history: List<dev.reins.core.ActivityEntry> = emptyList()): PendingDetails {
    val details = remember { PendingDetails() }
    val ids = pending.filter { it.kind == PendingKind.REQUEST }.map { it.id }
    val latest = androidx.compose.runtime.rememberUpdatedState(history)
    LaunchedEffect(ids) {
        for (id in ids) {
            if (id !in details.suggestions) details.suggestions[id] = quietly { container.core.autopilotSuggestion(id) }
            if (id !in details.views) (container.approvalViews[id] ?: quietly { container.core.approvalView(id) })?.let { details.views[id] = it }
        }
        if (ids.isEmpty()) return@LaunchedEffect
        val history = latest.value
        // The reason line: Autopilot's counts per kind, and for pushes the decided pushes of the same repo and
        // branch kind, read from their stored approvals (each entry once).
        val profiles = quietly { container.core.autopilotProfiles() }.orEmpty()
        val needPushes = ids.any { pushKey(details.views[it]) != null }
        if (needPushes) {
            history.asSequence().filter(::approvedPush).take(REASON_HISTORY).forEach { e ->
                if (e.id !in details.pushes) details.pushes[e.id] = pushKey(quietly { container.core.activityApproval(e.id) })
            }
        }
        val approved = history.filter(::approvedPush).take(REASON_HISTORY).mapNotNull { details.pushes[it.id] }
        for (id in ids) {
            val s = details.suggestions[id] ?: continue
            val cls = profiles.firstOrNull { it.id == s.profileId }?.classes?.firstOrNull { it.classKey == s.classKey }
            val item = pending.firstOrNull { it.id == id } ?: continue
            reasonLine(s, item, cls?.approved?.toInt(), cls?.denied?.toInt(), pushKey(details.views[id]), approved)?.let { details.reasons[id] = it } ?: details.reasons.remove(id)
        }
    }
    return details
}

/** How far back the reason line looks for pushes you approved. */
private const val REASON_HISTORY = 200

private suspend fun <T> quietly(block: suspend () -> T): T? = try {
    block()
} catch (e: CancellationException) {
    throw e
} catch (e: Exception) {
    null
}

/** A tone's colour on paper: its Signal field's kind ink, or ink-2 for [Tone.Other]. */
fun toneColor(tone: Tone, c: RColors): Color = tone.field?.kindInk(c) ?: c.ink2

/** Seconds left as a ring and a number ("40s", "1:20"), red for the last 15 s. Its clock recomposes only this. */
@Composable
fun TimerPill(createdAt: Long, waitUntil: Long?, now: State<Long>, modifier: Modifier = Modifier) {
    if (waitUntil == null) return
    val c = LocalColors.current
    val u by remember(createdAt, waitUntil, now) { derivedStateOf { urgency(createdAt, waitUntil, now.value) } }
    val left = u ?: return
    val hot = left.urgent
    val fg = when {
        left.stale -> c.tertiary
        hot -> c.danger
        else -> c.secondary
    }
    Row(
        modifier
            .background(if (hot) c.danger.copy(alpha = 0.13f) else c.controlFill, CircleShape)
            .padding(start = 8.dp, end = 11.dp, top = 7.dp, bottom = 7.dp)
            .testTag("timer"),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        ProgressRing(
            if (left.stale) 0f else left.fraction,
            if (hot) c.danger else c.secondary,
            if (hot) c.danger.copy(alpha = 0.22f) else c.controlFill.copy(alpha = 0.2f),
            size = 18.dp,
            stroke = 2.25.dp,
        )
        Spacer(Modifier.width(7.dp))
        val s = left.remainingSeconds
        RText(
            when {
                left.stale -> "Late"
                s < 60 -> "${s}s"
                else -> "${s / 60}:${(s % 60).toString().padStart(2, '0')}"
            },
            RType.sans(14f, FontWeight.SemiBold).copy(fontFeatureSettings = "tnum"),
            fg,
            maxLines = 1,
        )
    }
}

