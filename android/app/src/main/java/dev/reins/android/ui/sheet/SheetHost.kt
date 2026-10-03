package dev.reins.android.ui.sheet

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
