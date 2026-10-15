package dev.reins.android.ui.activity

import androidx.compose.animation.core.Animatable
import androidx.compose.animation.core.animate
import androidx.compose.animation.core.animateFloatAsState
import androidx.compose.animation.core.tween
import androidx.compose.foundation.gestures.Orientation
import androidx.compose.foundation.gestures.draggable
import androidx.compose.foundation.gestures.rememberDraggableState
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.BasicText
import androidx.compose.runtime.Composable
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.runtime.Immutable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.derivedStateOf
import androidx.compose.runtime.getValue
import androidx.compose.runtime.key
import androidx.compose.runtime.mutableFloatStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.rememberUpdatedState
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.TransformOrigin
import androidx.compose.ui.graphics.graphicsLayer
import androidx.compose.ui.layout.Layout
import androidx.compose.ui.layout.layout
import androidx.compose.ui.layout.onSizeChanged
import androidx.compose.ui.semantics.clearAndSetSemantics
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.Constraints
import androidx.compose.ui.unit.dp
import dev.reins.android.design.ButtonSize
import dev.reins.android.design.ButtonStyle
import dev.reins.android.design.Field
import dev.reins.android.design.Glyph
import dev.reins.android.design.LabelText
import dev.reins.android.design.LocalColors
import dev.reins.android.design.LocalReduceMotion
import dev.reins.android.design.Motion
import dev.reins.android.design.RColors
import dev.reins.android.design.RText
import dev.reins.android.design.RType
import dev.reins.android.design.Ring
import dev.reins.android.design.SignalButton
import dev.reins.android.design.askString
import dev.reins.android.design.fieldSurface
import dev.reins.android.design.pressable
import dev.reins.android.design.rememberNowState
import dev.reins.android.design.urgency
import dev.reins.android.feedback.Event
import dev.reins.android.feedback.LocalFeedback
import dev.reins.android.feedback.play
import dev.reins.android.ui.common.ConnectionIcon
import dev.reins.core.PendingItem
import dev.reins.core.PendingKind
import kotlin.math.abs
import kotlin.math.roundToInt
import kotlinx.coroutines.launch

// The stack (Reins Signal, iOS-Activity 01 / And-Core 01, Motion "The stack · flick" and "The fold"): waiting asks as
// cards in their field colour, newest on top, the rest peeking below. A flick right takes Autopilot's pick, left
// denies, a tap reviews; answered, the card folds away and lands at the top of the river.

/** What a card is about, for the stack: one request, or a burst of one AI. */
private val WaitCard.key: String get() = when (this) {
    is WaitCard.One -> "p:${item.id}"
    is WaitCard.Group -> "g:${burst.connectionId}"
}

private val WaitCard.first: PendingItem get() = when (this) {
    is WaitCard.One -> item
    is WaitCard.Group -> items.first()
}

/** How many cards show (the top one and two peeking). */
private const val LAYERS = 3

/** How far each card below peeks out, and how much narrower it is on each side. */
private val PEEK = 9.dp
private val INSET = 10.dp

/** A flick past this share of the card's width answers. */
private const val FLICK_SHARE = 0.35f

/** The tilt at a full card's width. */
private const val MAX_TILT = 6f

/**
 * The waiting stack. [onAnswer] answers a card through the same path as its buttons (Autopilot's Allow after the
 * owner check, or Deny); it returns when that is done, and a card it did not take comes back.
 */
@Composable
fun WaitingStack(
    cards: List<WaitCard>,
    details: PendingDetails,
    onReview: (PendingItem) -> Unit,
    onAnswer: suspend (Burst, Boolean) -> Unit,
    modifier: Modifier = Modifier,
) {
    val reduce = LocalReduceMotion.current
    // A card answered elsewhere (its button, the sheet, another device) folds away; a flicked one has already left.
    var departing by remember { mutableStateOf<WaitCard?>(null) }
    val flown = remember { mutableSetOf<String>() }
    var previous by remember { mutableStateOf(cards) }
    LaunchedEffect(cards) {
        val top = previous.firstOrNull()
        if (top != null && cards.none { it.key == top.key } && top.key !in flown) departing = top
        flown.retainAll(cards.map { it.key }.toSet())
        previous = cards
    }
    val shown = cards.take(LAYERS)
    Box(modifier.fillMaxWidth().padding(horizontal = 16.dp)) {
        if (shown.isNotEmpty()) {
            StackLayout(shown.size) {
                // Drawn bottom first: the top card is the last child.
                for (i in shown.indices.reversed()) {
                    val card = shown[i]
                    key(card.key) {
                        StackCard(
                            card,
                            depth = i,
                            details = details,
                            onReview = onReview,
                            onFlown = { flown += card.key },
                            onAnswer = onAnswer,
                        )
                    }
                }
            }
        }
        departing?.let { card ->
            key("departing:${card.key}") { FoldingCard(card, details, reduce) { departing = null } }
        }
    }
}

/**
 * Lays the cards out: the top one (the last child) at its own height, the ones below it at the same height (so they
 * peek out by [PEEK] each); the stack is as tall as the top card plus the peeks.
 */
@Composable
private fun StackLayout(count: Int, content: @Composable () -> Unit) {
    val peek = with(LocalDensity.current) { PEEK.roundToPx() }
    Layout(content) { measurables, constraints ->
        if (measurables.isEmpty()) return@Layout layout(constraints.minWidth, 0) {}
        val width = constraints.maxWidth
        val top = measurables.last().measure(Constraints(minWidth = width, maxWidth = width))
        val behind = measurables.dropLast(1).map { it.measure(Constraints.fixed(width, top.height)) }
        val height = top.height + peek * (count - 1)
        layout(width, height) {
            behind.forEach { it.place(0, 0) }
            top.place(0, 0)
        }
    }
}

/**
 * One card at [depth] (0 on top). Below the top it shows only its field, narrower and lower (it springs up when the
 * one above goes). On top it follows a horizontal drag, tilting up to 6°; past 35 % of its width it flies off in
 * 220 ms and answers; short of that it springs back.
 */
@Composable
private fun StackCard(
    card: WaitCard,
    depth: Int,
    details: PendingDetails,
    onReview: (PendingItem) -> Unit,
    onFlown: () -> Unit,
    onAnswer: suspend (Burst, Boolean) -> Unit,
) {
    val reduce = LocalReduceMotion.current
    val feedback = LocalFeedback.current
    val density = LocalDensity.current
    val scope = rememberCoroutineScope()
    val d by animateFloatAsState(depth.toFloat(), if (reduce) Motion.reduced() else Motion.cardSpring(), label = "depth")
    var dx by remember { mutableFloatStateOf(0f) }
    var widthPx by remember { mutableFloatStateOf(1f) }
    var past by remember { mutableStateOf(false) }
    val fade = remember { Animatable(1f) }
    val top = depth == 0
    val action = when (card) {
        is WaitCard.One -> waitAction(card.item, details.suggestions[card.item.id])
        is WaitCard.Group -> groupAction(card.items) { details.suggestions[it] }
    }
    val burst = when (card) {
        is WaitCard.One -> Burst(card.item.connectionId, card.item.connectionLabel, listOf(card.item.id), listOf(card.item.id))
        is WaitCard.Group -> card.burst
    }
    // Only requests are answered from the stack; pairings, new phones and files open their sheet.
    val flickable = top && card.first.kind == PendingKind.REQUEST
    val latestAction by rememberUpdatedState(action)
    val inset = with(density) { INSET.toPx() }
    val peek = with(density) { PEEK.toPx() }

    fun settle(velocity: Float) {
        val share = dx / widthPx
        val right = share > 0
        val answer: Boolean? = when {
            abs(share) < FLICK_SHARE && abs(velocity) < 2_500f -> null
            !right -> false
            latestAction == WaitAction.Allow -> true
            latestAction == WaitAction.Deny -> false
            else -> null
        }
        scope.launch {
            if (answer == null) {
                // Not far enough, or Autopilot has no pick to take: back on the stack (a right flick then reviews).
                animate(dx, 0f, animationSpec = Motion.cardSpring()) { v, _ -> dx = v }
                if (abs(share) >= FLICK_SHARE && right) onReview(card.first)
                return@launch
            }
            onFlown()
            if (reduce) {
                fade.animateTo(0f, tween(Motion.REDUCED_MS))
            } else {
                val target = (if (dx >= 0) 1 else -1) * widthPx * 1.4f
                animate(dx, target, animationSpec = tween(Motion.CARD_FLY_MS, easing = Motion.EaseOut)) { v, _ -> dx = v }
            }
            onAnswer(burst, answer)
            // Still here a moment later (the owner check was cancelled, or it failed): it comes back.
            kotlinx.coroutines.delay(150)
            fade.snapTo(1f)
            animate(dx, 0f, animationSpec = Motion.cardSpring()) { v, _ -> dx = v }
        }
    }

    val drag = rememberDraggableState { delta ->
        dx += delta
        val now = abs(dx / widthPx) >= FLICK_SHARE
        if (now != past) {
            past = now
            if (now) feedback.play(Event.MenuRow)
        }
    }
    Box(
        Modifier
            .onSizeChanged { widthPx = it.width.toFloat().coerceAtLeast(1f) }
            .graphicsLayer {
                // The cards below: narrower by INSET a side, lower by PEEK, all from the bottom edge.
                val s = 1f - (2 * inset * d) / widthPx
                scaleX = s
                scaleY = s
                transformOrigin = TransformOrigin(0.5f, 1f)
                translationY = peek * d
                translationX = dx
                rotationZ = (dx / widthPx).coerceIn(-1f, 1f) * MAX_TILT
                alpha = fade.value * if (d > 1.5f) 0.9f else 1f
            }
            .then(if (flickable) Modifier.draggable(drag, Orientation.Horizontal, onDragStopped = { v -> settle(v) }) else Modifier),
        propagateMinConstraints = true,
    ) {
        CardFace(card, details, showContent = top, action = action, onReview = { onReview(card.first) }) { approve ->
            // Answered by its button: it folds in place when it leaves the list, and the river takes it.
            scope.launch { onAnswer(burst, approve) }
        }
    }
}

/** A card answered while on top folds into its kind tile: it shrinks toward its top left and fades (360 ms). */
@Composable
private fun FoldingCard(card: WaitCard, details: PendingDetails, reduce: Boolean, onDone: () -> Unit) {
    val progress = remember { Animatable(0f) }
    LaunchedEffect(Unit) {
        progress.animateTo(1f, if (reduce) tween(Motion.REDUCED_MS) else tween(Motion.FOLD_MS, easing = Motion.EaseOut))
        onDone()
    }
    Box(
        Modifier
            // Already answered: nothing on it can be read out or pressed.
            .clearAndSetSemantics {}
            .layout { measurable, constraints ->
                val p = measurable.measure(constraints)
                // Its room closes as it folds, so the river rises under it at the same pace.
                val h = if (reduce) p.height else (p.height * (1f - progress.value)).roundToInt()
                layout(p.width, h) { p.place(0, 0) }
            }
            .graphicsLayer {
                val f = progress.value
                if (reduce) {
                    alpha = 1f - f
                } else {
                    transformOrigin = TransformOrigin(0.08f, 0.1f)
                    scaleX = 1f - 0.86f * f
                    scaleY = 1f - 0.8f * f
                    alpha = 1f - f * f
                }
            },
    ) { CardFace(card, details, showContent = true, action = WaitAction.Review, onReview = {}, onAnswer = {}) }
}

/**
 * A card's face: its field with the grain; on top, the label line (the AI's mark, what it touches and does, the
 * timer), the ask in the serif with its target in italic, who asks, Autopilot's reason, and Autopilot's pick beside
 * Review.
 */
@Composable
private fun CardFace(
    card: WaitCard,
    details: PendingDetails,
    showContent: Boolean,
    action: WaitAction,
    onReview: () -> Unit,
    onAnswer: (Boolean) -> Unit,
) {
    val c = LocalColors.current
    val first = card.first
    val field = when (card) {
        is WaitCard.One -> pendingTone(first, details.suggestions[first.id]).field
        is WaitCard.Group -> toneOf(if (card.items.all { it.action == first.action }) first.action else "search").field
    } ?: Field.Push
    val shape = RoundedCornerShape(26.dp)
    val tag = when (card) {
        is WaitCard.One -> "pending:${first.id}"
        is WaitCard.Group -> "burst"
    }
    Column(
        Modifier
            .fillMaxWidth()
            .heightIn(min = 196.dp)
            .fieldSurface(field, shape)
            .clip(shape)
            .then(if (showContent) Modifier.testTag(tag).pressable(shape = shape, label = "Review", onClick = onReview) else Modifier)
            .padding(start = 18.dp, end = 18.dp, top = 16.dp, bottom = 16.dp),
        verticalArrangement = Arrangement.SpaceBetween,
    ) {
        if (!showContent) return@Column
        Column(verticalArrangement = Arrangement.spacedBy(10.dp)) {
            Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(9.dp)) {
                if (first.kind != PendingKind.PAIRING && first.kind != PendingKind.JOIN) {
                    // The AI's mark on its white disc, the same in both themes.
                    CompositionLocalProvider(LocalColors provides RColors.Light) {
                        ConnectionIcon(first.connectionId, first.connectionLabel, size = 24.dp)
                    }
                }
                LabelText(cardLabel(first), c.onField2, Modifier.weight(1f))
                val waitUntil = when (card) {
                    is WaitCard.One -> first.waitUntil.takeIf { first.kind != PendingKind.BLOB }
                    is WaitCard.Group -> card.items.mapNotNull { it.waitUntil }.minOrNull()
                }
                if (waitUntil != null) CardTimer(first.createdAt, waitUntil)
            }
            val ask = when (card) {
                is WaitCard.One -> cardAsk(first, details.views[first.id])
                is WaitCard.Group -> groupAsk(card.items) { details.views[it] }
            }
            val askText = remember(ask) { askString(ask.text, ask.target) }
            BasicText(
                askText,
                Modifier.testTag(if (card is WaitCard.Group) "burstTitle" else "sentence:${first.id}"),
                style = RType.askSmall.copy(color = c.onField),
                maxLines = 3,
                overflow = TextOverflow.Ellipsis,
            )
            when (card) {
                is WaitCard.One -> {
                    RText(cardWho(first), RType.sans(13.5f), c.onField2, maxLines = 1)
                    details.reasons[first.id]?.let { RText(it, RType.sans(13.5f), c.onField2, Modifier.testTag("reason:${first.id}"), maxLines = 2) }
                }
                is WaitCard.Group -> {
                    RText(dev.reins.android.ui.common.untrusted(first.connectionLabel), RType.sans(13.5f), c.onField2, maxLines = 1)
                    card.items.take(3).forEach { item ->
                        RText("· " + groupLine(item, details.views[item.id]), RType.sans(13.5f), c.onField2, Modifier.testTag("burstItem:${item.id}"), maxLines = 1)
                    }
                    if (card.items.size > 3) RText("+${card.items.size - 3} more", RType.sans(13.5f), c.onField2, maxLines = 1)
                }
            }
        }
        Spacer(Modifier.height(14.dp))
        val count = if (card is WaitCard.Group) card.items.size else 1
        val percent = when (action) {
            WaitAction.Allow -> when (card) {
                is WaitCard.One -> details.suggestions[first.id]?.pApprove
                is WaitCard.Group -> card.items.mapNotNull { details.suggestions[it.id]?.pApprove }.minOrNull()
            }
            WaitAction.Deny -> when (card) {
                is WaitCard.One -> details.suggestions[first.id]?.pDeny
                is WaitCard.Group -> card.items.mapNotNull { details.suggestions[it.id]?.pDeny }.minOrNull()
            }
            else -> null
        }?.let { (it * 100).roundToInt() }?.takeIf { it > 0 }
        val one = card is WaitCard.One
        Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
            val many = if (count > 1) " $count" else ""
            val pct = percent?.let { " · $it%" }.orEmpty()
            when (action) {
                WaitAction.Allow -> SignalButton(
                    "Allow$many$pct", Modifier.weight(1.3f).testTag(if (one) "allow:${first.id}" else "approveAll"),
                    style = ButtonStyle.OnFieldWhite, size = ButtonSize.Small, glyph = Glyph.Fingerprint,
                ) { onAnswer(true) }
                WaitAction.Deny -> SignalButton(
                    "Deny$many$pct", Modifier.weight(1.3f).testTag(if (one) "deny:${first.id}" else "denyAll"),
                    style = ButtonStyle.OnFieldWhite, size = ButtonSize.Small, glyph = Glyph.Block,
                ) { onAnswer(false) }
                WaitAction.Unsure -> SignalButton(
                    "Unsure", Modifier.weight(1.3f).testTag("unsure:${if (one) first.id else "burst"}"),
                    style = ButtonStyle.OnFieldWash, size = ButtonSize.Small,
                    leading = { Ring(0.5f, c.onField, c.onField.copy(alpha = 0.35f), 16.dp) },
                    onClick = onReview,
                )
                WaitAction.AlwaysAsks -> SignalButton(
                    "Always asks", Modifier.weight(1.3f).testTag("alwaysAsks:${if (one) first.id else "burst"}"),
                    style = ButtonStyle.OnFieldWash, size = ButtonSize.Small, glyph = Glyph.Lock, onClick = onReview,
                )
                WaitAction.Review -> Unit
            }
            SignalButton(
                "Review",
                Modifier.weight(1f).testTag(if (one) "review:${first.id}" else "reviewBurst"),
                style = if (action == WaitAction.Review) ButtonStyle.OnFieldWhite else ButtonStyle.OnFieldWash,
                size = ButtonSize.Small,
                onClick = onReview,
            )
        }
    }
}

/** The time left on a card: the ring draining each second, a white pill with red digits under 15 s. Its own clock. */
@Composable
private fun CardTimer(createdAt: Long, waitUntil: Long) {
    val now = rememberNowState(1_000, untilMillis = waitUntil * 1000)
    val left by remember(createdAt, waitUntil) { derivedStateOf { (waitUntil - now.value / 1000).coerceAtLeast(0) } }
    val total = (waitUntil - createdAt).coerceAtLeast(1)
    if (urgency(createdAt, waitUntil, now.value)?.stale == true) {
        RText("Late", RType.mono(13f), LocalColors.current.onField, Modifier.testTag("timer"), maxLines = 1)
        return
    }
    dev.reins.android.design.TimerPill(left, total)
}
