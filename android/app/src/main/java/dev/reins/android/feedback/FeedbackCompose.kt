package dev.reins.android.feedback

import androidx.compose.runtime.Composable
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberUpdatedState
import androidx.compose.runtime.setValue
import androidx.compose.ui.platform.LocalView

/**
 * Installs the engine for a subtree: [LocalFeedback], and the window view for OEM-tuned haptics. Every `pressable`
 * control then answers with the default tap (see [TapFeedback]) unless its action asks for feedback of its own.
 */
@Composable
fun ProvideFeedback(feedback: AndroidFeedback, content: @Composable () -> Unit) {
    val view = LocalView.current
    DisposableEffect(feedback, view) {
        feedback.attachView(view)
        onDispose { feedback.attachView(null) }
    }
    CompositionLocalProvider(LocalFeedback provides feedback, content = content)
}

/** [action] preceded by [event]'s feedback: for `onClick =` parameters whose moment has a meaning of its own. */
@Composable
fun feedbackAction(event: Event, action: () -> Unit): () -> Unit {
    val fb = LocalFeedback.current
    val latest = rememberUpdatedState(action)
    return remember(fb, event) { { fb.play(event); latest.value() } }
}

/** The default tap after a press in a non-`pressable` control (a Material slider's track, a text link). */
fun Feedback.defaultTap() {
    if (this is TapFeedback) defaultTap() else play(Event.Tap)
}

/** [cue] unless another cue just played (the action that closed something already has its sound). */
fun Feedback.cueUnlessRecent(cue: Cue) {
    if (this is TapFeedback) cueUnlessRecent(cue) else cue(cue)
}

/**
 * For a sheet driven by a [shown] flag: [Cue.Open] as it appears, unless a chime just announced what it shows (a request
 * that pops up by itself). Closing is the caller's: a dismissal plays [Event.Close], a decision has its own sound.
 */
@Composable
fun SheetOpenFeedback(shown: Boolean) {
    val fb = LocalFeedback.current
    LaunchedEffect(shown) {
        if (shown) {
            if (fb is TapFeedback) fb.cueUnlessRecent(Cue.Open, ARRIVAL_WINDOW_MS) else fb.cue(Cue.Open)
        }
    }
}

/** For a dialog, placed in its content: [Cue.Open] as it appears, [Cue.Close] as it leaves (unless a choice just sounded). */
@Composable
fun DialogFeedback() {
    val fb = LocalFeedback.current
    DisposableEffect(fb) {
        fb.cue(Cue.Open)
        onDispose { fb.cueUnlessRecent(Cue.Close) }
    }
}

/**
 * A slider's `onValueChange` with detents: [Event.Detent] each time the value lands on another of its discrete steps,
 * climbing the scale as it goes up. [index] maps a value to its step.
 */
@Composable
fun detentAction(start: Int, index: (Float) -> Int, onChange: (Float) -> Unit): (Float) -> Unit {
    val fb = LocalFeedback.current
    val latest = rememberUpdatedState(onChange)
    var last by remember { mutableIntStateOf(start) }
    return { value ->
        latest.value(value)
        val step = index(value)
        if (step != last) {
            last = step
            fb.play(Event.Detent, step)
        }
    }
}

/** A sheet that opens this soon after another cue is announcing what that cue already announced. */
private const val ARRIVAL_WINDOW_MS = 700L
