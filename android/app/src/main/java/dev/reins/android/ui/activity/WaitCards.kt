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
}

@Composable
fun rememberPendingDetails(container: AppContainer, pending: List<PendingItem>): PendingDetails {
    val details = remember { PendingDetails() }
    val ids = pending.filter { it.kind == PendingKind.REQUEST }.map { it.id }
    LaunchedEffect(ids) {
        for (id in ids) {
            if (id !in details.suggestions) details.suggestions[id] = quietly { container.core.autopilotSuggestion(id) }
            if (id !in details.views) (container.approvalViews[id] ?: quietly { container.core.approvalView(id) })?.let { details.views[id] = it }
        }
    }
    return details
}

private suspend fun <T> quietly(block: suspend () -> T): T? = try {
    block()
} catch (e: CancellationException) {
    throw e
} catch (e: Exception) {
    null
}

fun toneColor(tone: Tone, c: RColors): Color = when (tone) {
    Tone.Search -> c.search
    Tone.Read -> c.read
    Tone.Write -> c.send
    Tone.Danger -> c.danger
    Tone.Grant -> c.accent
    Tone.Pair -> c.pair
    Tone.Other -> c.secondary
}

/** A sentence with its key nouns in the text colour and the rest quieter. */
@Composable
fun SentenceText(sentence: Sentence, modifier: Modifier = Modifier, size: Float = 16f, lineHeight: Float = 21f, maxLines: Int = 3, align: androidx.compose.ui.text.style.TextAlign? = null) {
    val c = LocalColors.current
    val text = remember(sentence, c) {
        buildAnnotatedString {
            sentence.parts.forEach { part ->
                if (part.bold) withStyle(SpanStyle(color = c.text, fontWeight = FontWeight.SemiBold)) { append(part.text) } else append(part.text)
            }
        }
    }
    androidx.compose.material3.Text(
        text,
        modifier,
        color = c.secondary,
        style = RType.sans(size, FontWeight.Medium, lineHeight = lineHeight),
        maxLines = maxLines,
        overflow = androidx.compose.ui.text.style.TextOverflow.Ellipsis,
        textAlign = align,
    )
}

/** The AI's face and the service it asks of, for a waiting request. */
@Composable
fun PendingMini(item: PendingItem, ring: Color) {
    val c = LocalColors.current
    val fallback = when (item.kind) {
        PendingKind.PAIRING -> Icons.Rounded.Computer
        PendingKind.JOIN -> Icons.Rounded.PhoneAndroid
        PendingKind.BLOB -> Icons.Rounded.UploadFile
        else -> null
    }
    val fromAi = item.kind != PendingKind.PAIRING && item.kind != PendingKind.JOIN
    MiniLogo(
        service = if (fromAi) item.service.ifBlank { "gmail" } else "",
        face = { size -> if (fromAi) ConnectionIcon(item.connectionId, item.connectionLabel, size = size) },
        ring = ring,
        fallback = fallback,
        fallbackTint = c.pair,
    )
}

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

/** A waiting request: what it wants in one sentence, its timer, and Autopilot's pick beside Review. */
@Composable
fun WaitingCard(
    item: PendingItem,
    suggestion: SuggestionView?,
    view: ApprovalView?,
    modifier: Modifier = Modifier,
    onReview: () -> Unit,
    onAllow: () -> Unit,
    onDeny: () -> Unit,
) {
    val c = LocalColors.current
    val tone = toneColor(pendingTone(item, suggestion), c)
    // An upload waits until the server deletes it, not for an AI that is holding on: no countdown.
    val waitUntil = if (item.kind == PendingKind.BLOB) null else item.waitUntil
    val now = rememberNowState(untilMillis = waitUntil?.let { it * 1000 })
    val sentence = remember(item, view) { pendingSentence(item, view) }
    Column(
        modifier
            .fillMaxWidth()
            .toneCard(tone, top = if (c.dark) 0.16f else 0.10f, bottom = if (c.dark) 0.04f else 0.03f)
            .testTag("pending:${item.id}")
            .pressable(highlight = c.controlFill, shape = RoundedCornerShape(24.dp), onClick = onReview)
            .padding(14.dp),
    ) {
        Row(verticalAlignment = Alignment.CenterVertically) {
            PendingMini(item, ring = mixOver(tone, c))
            Spacer(Modifier.width(14.dp))
            SentenceText(sentence, Modifier.weight(1f).testTag("sentence:${item.id}"))
            Spacer(Modifier.width(8.dp))
            TimerPill(item.createdAt, waitUntil, now)
        }
        Spacer(Modifier.height(14.dp))
        ActionRow(waitAction(item, suggestion), count = 1, confidence = suggestion?.pApprove ?: 0f, tag = item.id, onReview = onReview, onAllow = onAllow, onDeny = onDeny)
    }
}

/** Routine requests of one AI as one card: what they are together, a line each, and one answer for all of them. */
@Composable
fun GroupCard(
    group: WaitCard.Group,
    details: PendingDetails,
    modifier: Modifier = Modifier,
    onReview: () -> Unit,
    onAllow: () -> Unit,
    onDeny: () -> Unit,
) {
    val c = LocalColors.current
    val first = group.items.first()
    val action = groupAction(group.items) { details.suggestions[it] }
    val tone = toneColor(toneOf(if (group.items.all { it.action == first.action }) first.action else "search"), c)
    val waitUntil = group.items.mapNotNull { it.waitUntil }.minOrNull()
    val createdAt = group.items.minOf { it.createdAt }
    val now = rememberNowState(untilMillis = waitUntil?.let { it * 1000 })
    val sentence = groupSentence(group.items) { details.views[it] }
    Column(
        modifier
            .fillMaxWidth()
            .toneCard(tone, top = if (c.dark) 0.16f else 0.10f, bottom = if (c.dark) 0.04f else 0.03f)
            .testTag("burst")
            .pressable(highlight = c.controlFill, shape = RoundedCornerShape(24.dp), onClick = onReview)
            .padding(14.dp),
    ) {
        Row(verticalAlignment = Alignment.CenterVertically) {
            PendingMini(first, ring = mixOver(tone, c))
            Spacer(Modifier.width(14.dp))
            SentenceText(sentence, Modifier.weight(1f).testTag("burstTitle"))
            Spacer(Modifier.width(8.dp))
            TimerPill(createdAt, waitUntil, now)
        }
        Column(Modifier.padding(start = 58.dp, top = 10.dp), verticalArrangement = Arrangement.spacedBy(5.dp)) {
            group.items.take(MAX_LINES).forEach { item ->
                Row(verticalAlignment = Alignment.CenterVertically, modifier = Modifier.testTag("burstItem:${item.id}")) {
                    Icon(Icons.Rounded.MailOutline.takeIf { item.service.isBlank() || item.service == "gmail" } ?: dev.reins.android.design.ActionKind.of(item.action).icon, null, Modifier.size(14.dp), tint = c.tertiary)
                    Spacer(Modifier.width(7.dp))
                    RText(groupLine(item, details.views[item.id]), RType.sans(13f), c.secondary, maxLines = 1)
                }
            }
            if (group.items.size > MAX_LINES) RText("+${group.items.size - MAX_LINES} more", RType.sans(13f), c.tertiary, maxLines = 1)
        }
        Spacer(Modifier.height(14.dp))
        ActionRow(action, count = group.items.size, confidence = group.items.mapNotNull { details.suggestions[it.id]?.pApprove }.minOrNull() ?: 0f, tag = "burst", onReview = onReview, onAllow = onAllow, onDeny = onDeny)
    }
}

private const val MAX_LINES = 4

/** The colour of the card behind the face's ring: the tone over the page, roughly. */
private fun mixOver(tone: Color, c: RColors): Color = androidx.compose.ui.graphics.lerp(c.background, tone, 0.14f)

/** Autopilot's pick (or a plain label) on the left, Review on the right. */
@Composable
private fun ActionRow(
    action: WaitAction,
    count: Int,
    confidence: Float,
    tag: String,
    onReview: () -> Unit,
    onAllow: () -> Unit,
    onDeny: () -> Unit,
) {
    val c = LocalColors.current
    val (reviewBg, reviewFg) = primaryFill(c)
    val many = if (count > 1) " $count" else ""
    Row(Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.spacedBy(8.dp), verticalAlignment = Alignment.CenterVertically) {
        when (action) {
            WaitAction.Allow -> PillButton(
                "Allow$many", Modifier.weight(1f).testTag(if (tag == "burst") "approveAll" else "allow:$tag"), icon = Icons.Rounded.Fingerprint,
                background = c.success, foreground = onSuccess(c), height = 42.dp, onClick = onAllow,
            )
            WaitAction.Deny -> PillButton(
                "Deny$many", Modifier.weight(1f).testTag(if (tag == "burst") "denyAll" else "deny:$tag"), icon = Icons.Rounded.Block,
                background = c.danger.copy(alpha = 0.15f), foreground = c.danger, height = 42.dp, onClick = onDeny,
            )
            WaitAction.Unsure -> Row(Modifier.weight(1f).semantics(mergeDescendants = true) {}.testTag("unsure:$tag"), horizontalArrangement = Arrangement.Center, verticalAlignment = Alignment.CenterVertically) {
                ProgressRing(confidence, c.search, c.search.copy(alpha = 0.2f), size = 16.dp, stroke = 2.dp)
                Spacer(Modifier.width(7.dp))
                RText("Unsure", RType.sans(14f, FontWeight.SemiBold), c.secondary, maxLines = 1)
            }
            WaitAction.AlwaysAsks -> Row(Modifier.weight(1f).semantics(mergeDescendants = true) {}.testTag("alwaysAsks:$tag"), horizontalArrangement = Arrangement.Center, verticalAlignment = Alignment.CenterVertically) {
                Icon(Icons.Rounded.Lock, null, Modifier.size(15.dp), tint = c.secondary)
                Spacer(Modifier.width(6.dp))
                RText("Always asks", RType.sans(14f, FontWeight.SemiBold), c.secondary, maxLines = 1)
            }
            WaitAction.Review -> Unit
        }
        val weight = when (action) {
            WaitAction.Unsure, WaitAction.AlwaysAsks -> 2f
            else -> 1f
        }
        PillButton(
            "Review", Modifier.weight(weight).testTag(if (tag == "burst") "reviewBurst" else "review:$tag"),
            background = reviewBg, foreground = reviewFg, height = 42.dp, onClick = onReview,
        )
    }
}
