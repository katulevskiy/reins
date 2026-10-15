package dev.reins.android.ui.sheet

import androidx.compose.runtime.LaunchedEffect
import androidx.compose.ui.graphics.graphicsLayer

import androidx.activity.compose.BackHandler
import androidx.compose.animation.AnimatedVisibility
import androidx.compose.animation.core.tween
import androidx.compose.animation.fadeIn
import androidx.compose.animation.fadeOut
import androidx.compose.animation.slideInVertically
import androidx.compose.animation.slideOutVertically
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.interaction.MutableInteractionSource
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxHeight
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.unit.dp
import dev.reins.android.design.Glyph
import dev.reins.android.design.LocalColors
import dev.reins.android.design.CircleIconButton
import dev.reins.android.feedback.Event
import dev.reins.android.feedback.LocalFeedback
import dev.reins.android.feedback.SheetOpenFeedback
import dev.reins.android.feedback.play

/**
 * A sheet that covers about 80% of the app, over whatever is showing. Tapping outside, the close button or Back
 * dismisses it; the item it shows simply stays in the list.
 */
@Composable
fun <T : Any> SheetHost(target: T?, onClose: () -> Unit, content: @Composable (T) -> Unit) {
    val c = LocalColors.current
    val feedback = LocalFeedback.current
    // Keep the last target on screen while the sheet slides away.
    var shown by remember { mutableStateOf(target) }
    if (target != null) shown = target
    SheetOpenFeedback(target != null)
    // Dismissing sounds; a decision that closes the sheet (approve, deny) has already made its own.
    val dismiss = {
        feedback.play(Event.Close)
        onClose()
    }
    BackHandler(enabled = target != null, onBack = dismiss)

    Box(Modifier.fillMaxSize()) {
        AnimatedVisibility(target != null, enter = fadeIn(tween(180)), exit = fadeOut(tween(160))) {
            Box(
                Modifier
                    .fillMaxSize()
                    .background(c.scrim)
                    .clickable(interactionSource = remember { MutableInteractionSource() }, indication = null, onClick = dismiss),
            )
        }
        AnimatedVisibility(
            target != null,
            modifier = Modifier.align(Alignment.BottomCenter),
            enter = slideInVertically(tween(260)) { it },
            exit = slideOutVertically(tween(200)) { it },
        ) {
            val shape = RoundedCornerShape(topStart = 30.dp, topEnd = 30.dp)
            Box(
                Modifier
                    .fillMaxWidth()
                    .fillMaxHeight(0.82f)
                    .background(c.background, shape)
                    .border(0.75.dp, c.glassEdge, shape)
                    .testTag("sheet"),
            ) {
                shown?.let { item ->
                    Column(Modifier.fillMaxSize()) {
                        Box(Modifier.fillMaxWidth().padding(top = 8.dp), contentAlignment = Alignment.TopCenter) {
                            Box(Modifier.width(38.dp).height(4.dp).background(c.tertiary.copy(alpha = 0.5f), CircleShape))
                        }
                        Box(Modifier.weight(1f).fillMaxWidth()) { content(item) }
                    }
                    CircleIconButton(
                        Glyph.Close,
                        Modifier.align(Alignment.TopEnd).padding(top = 14.dp, end = 14.dp).testTag("closeSheet"),
                        size = 36.dp,
                        iconSize = 15.dp,
                        label = "Close",
                        onClick = dismiss,
                    )
                }
            }
        }
    }
}

/**
 * An ask over the whole app (an approval, a pairing, a new phone, a file): its paper appears and its field slides
 * down from the top by itself (it does not rise from the bottom). Back means "not now": the predictive back gesture
 * shrinks the screen toward the edge it comes from as a preview, and letting go dismisses it without answering (the
 * item stays waiting). A page pushed inside it, or a sheet over it, handles Back first.
 */
@Composable
fun <T : Any> FullScreenHost(target: T?, onClose: () -> Unit, content: @Composable (T) -> Unit) {
    val feedback = LocalFeedback.current
    val reduce = dev.reins.android.design.LocalReduceMotion.current
    var shown by remember { mutableStateOf(target) }
    if (target != null) shown = target
    SheetOpenFeedback(target != null)
    // How far the back gesture has gone (0..1) and from which edge.
    var back by remember { mutableStateOf(0f) }
    var fromLeft by remember { mutableStateOf(true) }
    androidx.activity.compose.PredictiveBackHandler(enabled = target != null) { events ->
        try {
            events.collect { e ->
                back = e.progress
                fromLeft = e.swipeEdge == androidx.activity.BackEventCompat.EDGE_LEFT
            }
            feedback.play(Event.Close)
            onClose()
        } catch (e: kotlin.coroutines.cancellation.CancellationException) {
            // Let go short of the end: it stays.
            back = 0f
            throw e
        }
    }
    LaunchedEffect(target) { if (target == null) back = 0f }
    AnimatedVisibility(
        target != null,
        enter = fadeIn(tween(if (reduce) dev.reins.android.design.Motion.REDUCED_MS else 120)),
        exit = fadeOut(tween(if (reduce) dev.reins.android.design.Motion.REDUCED_MS else 180)),
    ) {
        Box(
            Modifier
                .fillMaxSize()
                .graphicsLayer {
                    // The preview of "not now": smaller, toward the edge the gesture comes from, with round corners.
                    val p = back
                    val s = 1f - 0.08f * p
                    scaleX = s
                    scaleY = s
                    transformOrigin = androidx.compose.ui.graphics.TransformOrigin(if (fromLeft) 1f else 0f, 0.5f)
                    translationX = (if (fromLeft) 1f else -1f) * 14.dp.toPx() * p
                    shape = RoundedCornerShape((34 * p).dp)
                    clip = p > 0f
                }
                .testTag("sheet"),
        ) { shown?.let { content(it) } }
    }
}
