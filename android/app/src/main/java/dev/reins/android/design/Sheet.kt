package dev.reins.android.design

import androidx.activity.compose.BackHandler
import androidx.compose.animation.AnimatedVisibility
import androidx.compose.animation.core.tween
import androidx.compose.animation.fadeIn
import androidx.compose.animation.fadeOut
import androidx.compose.animation.slideInVertically
import androidx.compose.animation.slideOutVertically
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.gestures.detectTapGestures
import androidx.compose.foundation.interaction.MutableInteractionSource
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.BoxWithConstraints
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.ColumnScope
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.WindowInsets
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.navigationBars
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.layout.windowInsetsPadding
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.verticalScroll
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.draw.shadow
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.semantics.heading
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.unit.dp
import dev.reins.android.feedback.Event
import dev.reins.android.feedback.LocalFeedback
import dev.reins.android.feedback.SheetOpenFeedback
import dev.reins.android.feedback.play

/**
 * A Next bottom sheet (`.sheet` on Android): over a scrim, raised paper (or [paper]) with 28 dp top corners and a
 * 32 × 4 grabber, a display title (`--s 30`) with a round close button, then [content]. Tapping the scrim, the close
 * button or Back dismisses it. Put it last in a full-screen box.
 */
@Composable
fun NextSheet(
    open: Boolean,
    onClose: () -> Unit,
    modifier: Modifier = Modifier,
    title: String? = null,
    paper: Boolean = false,
    tag: String = "nextSheet",
    content: @Composable ColumnScope.() -> Unit,
) {
    val c = LocalColors.current
    val feedback = LocalFeedback.current
    val reduce = LocalReduceMotion.current
    // Keep what was shown on screen while the sheet slides away.
    var shownTitle by remember { mutableStateOf(title) }
    if (open) shownTitle = title
    SheetOpenFeedback(open)
    val dismiss = {
        feedback.play(Event.Close)
        onClose()
    }
    BackHandler(enabled = open, onBack = dismiss)
    BoxWithConstraints(modifier.fillMaxSize()) {
        val tallest = maxHeight * 0.92f
        AnimatedVisibility(open, enter = fadeIn(tween(180)), exit = fadeOut(tween(160))) {
            Box(
                Modifier
                    .fillMaxSize()
                    .background(c.scrim)
                    .clickable(interactionSource = remember { MutableInteractionSource() }, indication = null, onClick = dismiss),
            )
        }
        AnimatedVisibility(
            open,
            modifier = Modifier.align(Alignment.BottomCenter),
            enter = if (reduce) fadeIn(tween(Motion.REDUCED_MS)) else slideInVertically(tween(260)) { it },
            exit = if (reduce) fadeOut(tween(Motion.REDUCED_MS)) else slideOutVertically(tween(200)) { it },
        ) {
            val shape = RoundedCornerShape(topStart = 28.dp, topEnd = 28.dp)
            Column(
                Modifier
                    .fillMaxWidth()
                    .heightIn(max = tallest)
                    .shadow(24.dp, shape, ambientColor = Color.Black.copy(alpha = 0.3f), spotColor = Color.Black.copy(alpha = 0.3f))
                    .clip(shape)
                    .background(if (paper) c.paper else c.paperRaised)
                    // Taps on the sheet stay on it (no semantics, so the content keeps its own).
                    .pointerInput(Unit) { detectTapGestures {} }
                    .windowInsetsPadding(WindowInsets.navigationBars)
                    .padding(top = 8.dp, bottom = 24.dp)
                    .testTag(tag),
            ) {
                Box(Modifier.fillMaxWidth().padding(bottom = 6.dp), contentAlignment = Alignment.Center) {
                    Box(Modifier.width(32.dp).height(4.dp).clip(CircleShape).background(c.inkFaint.copy(alpha = 0.6f)))
                }
                shownTitle?.let { SheetTitle(it, onClose = dismiss) }
                Column(Modifier.fillMaxWidth().verticalScroll(rememberScrollState()), content = content)
            }
        }
    }
}

/** A sheet's title row: the display title (`--s 30`) and the round close button. */
@Composable
fun SheetTitle(title: String, onClose: () -> Unit, modifier: Modifier = Modifier) {
    val c = LocalColors.current
    Row(
        modifier.fillMaxWidth().padding(start = 20.dp, end = 16.dp, top = 10.dp, bottom = 14.dp),
        horizontalArrangement = Arrangement.spacedBy(12.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        RText(title, RType.sheetTitle, c.ink, Modifier.weight(1f).semantics { heading() }, maxLines = 2)
        RoundIconButton(Glyph.Close, "Close", Modifier.testTag("closeSheet"), onClick = onClose)
    }
}
