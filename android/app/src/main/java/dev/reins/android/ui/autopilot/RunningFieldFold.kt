package dev.reins.android.ui.autopilot

import androidx.compose.animation.Crossfade
import androidx.compose.animation.core.tween
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.WindowInsets
import androidx.compose.foundation.layout.statusBars
import androidx.compose.foundation.layout.union
import androidx.compose.runtime.Composable
import androidx.compose.runtime.Stable
import androidx.compose.runtime.derivedStateOf
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberUpdatedState
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.TransformOrigin
import androidx.compose.ui.layout.Layout
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.unit.Constraints
import androidx.compose.ui.unit.dp
import dev.reins.android.design.LocalReduceMotion
import dev.reins.android.design.Motion
import kotlin.math.max
import kotlin.math.min
import kotlin.math.roundToInt

// A running bypass, Overnight or lockdown folds into a compact bar as Trust scrolls: the label line and the timer stay,
// the sentence under them fades and closes, and the button slides up to the right of the timer. Everything follows the
// scroll offset continuously (no snap); under Reduce Motion the two states cross-fade instead.
//
// The field always takes its open height (Trust keeps that much room above its content), so the page's layout never
// changes while it scrolls. The colour is drawn at the current height and the page scrolls on under the rest: while
// the field folds, the page's first row stays right under its edge.

/** The field's two heights, in px from the top of the screen; written by its layout, read by Trust for its room. */
@Stable
class RunningFold {
    var open by mutableIntStateOf(0)
    var compact by mutableIntStateOf(0)

    /** The scroll it takes to fold the field: as far as its edge moves. */
    val range: Int get() = max(open - compact, 0)

    companion object {
        /** 0 open … 1 compact: the share of [range] scrolled; under Reduce Motion one state or the other, switching halfway. */
        fun progress(scrolled: Int, range: Int, reduceMotion: Boolean): Float {
            if (range <= 1) return 0f
            val p = (scrolled.toFloat() / range).coerceIn(0f, 1f)
            return if (reduceMotion) (if (p >= 0.5f) 1f else 0f) else p
        }

        /** The sentence under the timer fades out over the first part of the fold, before the button crosses it. */
        fun heroAlpha(progress: Float): Float = (1f - progress / 0.45f).coerceIn(0f, 1f)

        /** Test tag of the field's state: "trustField:open", "trustField:folding", "trustField:compact". */
        fun stateTag(progress: Float): String = when {
            progress >= 1f -> "trustField:compact"
            progress <= 0f -> "trustField:open"
            else -> "trustField:folding"
        }
    }
}

/** Where the field's parts go at a progress between open (0) and compact (1), in px. */
internal data class FoldGeometry(
    val labelY: Int,
    val labelWidth: Int,
    val clockY: Int,
    /** The timer is drawn scaled by this from its top-left corner. */
    val clockScale: Float,
    val heroY: Int,
    val buttonX: Int,
    val buttonY: Int,
    val buttonWidth: Int,
    val buttonHeight: Int,
    /** The coloured surface: the height the field shows now, status bar included. */
    val surfaceHeight: Int,
    val openHeight: Int,
    val compactHeight: Int,
) {
    /** Measured sizes and the spacing, in px. */
    data class Sizes(
        val width: Int,
        /** Where the content starts: the status bar and the 8 dp under it. */
        val top: Int,
        val side: Int,
        val gap: Int,
        val labelHeight: Int,
        /** The label line's own width (its words and glyphs). */
        val labelNaturalWidth: Int,
        /** Null without a timer (a lockdown, or a mode without an end). */
        val clockWidth: Int?,
        val clockHeight: Int?,
        val heroHeight: Int,
        val buttonCompactWidth: Int,
        val openButton: Int,
        val compactButton: Int,
        val openBottom: Int,
        val compactBottom: Int,
    )

    companion object {
        /** The timer compact: 58 sp serif drawn at about 30. */
        const val CLOCK_SCALE = 30f / 58f

        fun at(s: Sizes, progress: Float): FoldGeometry {
            val p = progress.coerceIn(0f, 1f)
            val inner = max(s.width - 2 * s.side, 0)
            val buttonW = min(s.buttonCompactWidth, inner)
            val hasClock = s.clockHeight != null && s.clockWidth != null

            // Open: the label line, the timer, the sentence and the line under it, the button across.
            val clockOpenY = s.top + s.labelHeight + s.gap
            val heroOpenY = (if (hasClock) clockOpenY + s.clockHeight!! else s.top + s.labelHeight) + s.gap
            val buttonOpenY = heroOpenY + s.heroHeight + s.gap + s.gap / 3
            val openH = buttonOpenY + s.openButton + s.openBottom

            // Compact: the label line, then the timer with the button on its right. Without a timer the button sits on
            // the label line when both fit, else right-aligned under it (the label is never cut).
            val room = inner - buttonW - s.gap
            val scale = if (hasClock) min(CLOCK_SCALE, room.toFloat() / s.clockWidth!!.coerceAtLeast(1)) else 1f
            val beside = !hasClock && s.labelNaturalWidth + s.gap + buttonW <= inner
            val rowTop: Int
            val rowH: Int
            val labelCompactY: Int
            var clockCompactY = 0
            if (!beside) {
                val drawn = if (hasClock) (s.clockHeight!! * scale).roundToInt() else 0
                rowTop = s.top + s.labelHeight + s.gap / 2
                rowH = max(drawn, s.compactButton)
                labelCompactY = s.top
                clockCompactY = rowTop + (rowH - drawn) / 2
            } else {
                rowTop = s.top
                rowH = max(s.labelHeight, s.compactButton)
                labelCompactY = s.top + (rowH - s.labelHeight) / 2
            }
            val buttonCompactY = rowTop + (rowH - s.compactButton) / 2
            val compactH = rowTop + rowH + s.compactBottom

            return FoldGeometry(
                labelY = lerp(s.top, labelCompactY, p),
                labelWidth = inner - if (beside) (p * (buttonW + s.gap)).roundToInt() else 0,
                clockY = lerp(clockOpenY, clockCompactY, p),
                clockScale = 1f + (scale - 1f) * p,
                heroY = lerp(heroOpenY, rowTop, p),
                buttonX = lerp(s.side, s.width - s.side - buttonW, p),
                buttonY = lerp(buttonOpenY, buttonCompactY, p),
                buttonWidth = lerp(inner, buttonW, p),
                buttonHeight = lerp(s.openButton, s.compactButton, p),
                surfaceHeight = lerp(openH, compactH, p),
                openHeight = openH,
                compactHeight = compactH,
            )
        }

        private fun lerp(a: Int, b: Int, t: Float): Int = (a + (b - a) * t).roundToInt()
    }
}

/**
 * The running field, folding with [scrolled] (the page's scroll in px). Its parts: the coloured [surface], the [label]
 * line, the [clock] (none for a lockdown or a mode without an end), the [hero] sentence with its line, and the [button].
 * It always measures at its open height; under Reduce Motion it cross-fades between open and compact.
 */
@Composable
internal fun FoldingField(
    fold: RunningFold,
    scrolled: () -> Int,
    surface: @Composable () -> Unit,
    label: @Composable () -> Unit,
    clock: (@Composable () -> Unit)?,
    hero: @Composable () -> Unit,
    button: @Composable () -> Unit,
    modifier: Modifier = Modifier,
) {
    val reduce = LocalReduceMotion.current
    Box(modifier.testTag("trustField")) {
        if (reduce) {
            val compact by remember(fold) { derivedStateOf { RunningFold.progress(scrolled(), fold.range, reduceMotion = true) >= 1f } }
            Crossfade(compact, animationSpec = tween(Motion.FADE_MS), label = "foldFade") { c ->
                FoldLayout(fold, { if (c) 1f else 0f }, surface, label, clock, hero, button)
            }
        } else {
            FoldLayout(fold, { RunningFold.progress(scrolled(), fold.range, reduceMotion = false) }, surface, label, clock, hero, button)
        }
    }
}

@Composable
private fun FoldLayout(
    fold: RunningFold,
    progress: () -> Float,
    surface: @Composable () -> Unit,
    label: @Composable () -> Unit,
    clock: (@Composable () -> Unit)?,
    hero: @Composable () -> Unit,
    button: @Composable () -> Unit,
) {
    val density = LocalDensity.current
    // Never shorter than the boards' 36 dp status bar (a renderer without system bars reports none).
    val statusTop = WindowInsets.statusBars.union(WindowInsets(top = 36.dp)).getTop(density)
    // Its state for tests (what TalkBack reads is unchanged); recomposes only when the state changes, while the
    // layout itself follows every scrolled pixel without recomposing.
    val current by rememberUpdatedState(progress)
    val tag by remember { derivedStateOf { RunningFold.stateTag(current()) } }
    Layout(
        contents = listOf(surface, label, clock ?: {}, hero, button),
        modifier = Modifier.testTag(tag),
    ) { (surfaceM, labelM, clockM, heroM, buttonM), constraints ->
        val width = constraints.maxWidth
        val side = 24.dp.roundToPx()
        val inner = max(width - 2 * side, 0)
        val compactButton = 44.dp.roundToPx()
        val buttonCompactWidth = buttonM.first().maxIntrinsicWidth(compactButton)
        val labelHeight = labelM.first().minIntrinsicHeight(inner)
        val clockP = clockM.firstOrNull()?.measure(Constraints(maxWidth = inner))
        val heroP = heroM.first().measure(Constraints.fixedWidth(inner))
        val sizes = FoldGeometry.Sizes(
            width = width,
            top = statusTop + 8.dp.roundToPx(),
            side = side,
            gap = 12.dp.roundToPx(),
            labelHeight = labelHeight,
            labelNaturalWidth = labelM.first().maxIntrinsicWidth(labelHeight),
            clockWidth = clockP?.width,
            clockHeight = clockP?.height,
            heroHeight = heroP.height,
            buttonCompactWidth = buttonCompactWidth,
            openButton = 54.dp.roundToPx(),
            compactButton = compactButton,
            openBottom = 24.dp.roundToPx(),
            compactBottom = 12.dp.roundToPx(),
        )
        val ends = FoldGeometry.at(sizes, 0f)
        if (fold.open != ends.openHeight) fold.open = ends.openHeight
        if (fold.compact != ends.compactHeight) fold.compact = ends.compactHeight
        val progressNow = current()
        val g = FoldGeometry.at(sizes, progressNow)
        val surfaceP = surfaceM.first().measure(Constraints.fixed(width, g.surfaceHeight))
        val labelP = labelM.first().measure(Constraints.fixedWidth(max(g.labelWidth, 0)))
        val buttonP = buttonM.first().measure(Constraints.fixed(g.buttonWidth, g.buttonHeight))
        val alpha = RunningFold.heroAlpha(progressNow)
        layout(width, g.openHeight) {
            surfaceP.place(0, 0)
            labelP.place(side, g.labelY)
            clockP?.placeWithLayer(side, g.clockY) {
                scaleX = g.clockScale
                scaleY = g.clockScale
                transformOrigin = TransformOrigin(0f, 0f)
            }
            // Faded, not removed: TalkBack still reads it when compact.
            heroP.placeWithLayer(side, g.heroY) { this.alpha = alpha }
            buttonP.place(g.buttonX, g.buttonY)
        }
    }
}
