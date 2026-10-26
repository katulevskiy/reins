package dev.reins.android.ui.approval

import androidx.compose.ui.semantics.clearAndSetSemantics
import androidx.compose.ui.semantics.testTag
import androidx.compose.ui.semantics.text

import androidx.compose.animation.AnimatedVisibility
import androidx.compose.animation.core.MutableTransitionState
import androidx.compose.animation.core.tween
import androidx.compose.animation.fadeIn
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.ColumnScope
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.RowScope
import androidx.compose.foundation.layout.WindowInsets
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.navigationBars
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.windowInsetsPadding
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.runtime.Composable
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.runtime.remember
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.draw.drawBehind
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp
import dev.reins.android.design.AskText
import dev.reins.android.design.Field
import dev.reins.android.design.FieldHeaderShape
import dev.reins.android.design.Glyph
import dev.reins.android.design.GlyphIcon
import dev.reins.android.design.LabelText
import dev.reins.android.design.LocalColors
import dev.reins.android.design.LocalReduceMotion
import dev.reins.android.design.Motion
import dev.reins.android.design.RColors
import dev.reins.android.design.RText
import dev.reins.android.design.RType
import dev.reins.android.design.askString
import dev.reins.android.design.fieldSurface
import dev.reins.android.design.pressable
import dev.reins.android.ui.main.HeaderInsets

// The ask as a field (Reins Signal, And-Core 02, iOS-Approve, iOS-Kinds, iOS-Connect): a solid colour that runs into the
// top of the screen, with who asks and how long it waits, the label line, the ask in the serif with its target in
// italic, and one verdict line. It slides down from the top (320 ms, ease out); the ask follows 60 ms later.

/**
 * A field at the top of an ask (Next, Approve and Every kind): the timer and the close button, then what it touches
 * and who asks ([icon] · [top]), the ask in display type with its target in 600, the machine detail in mono, and
 * [verdict]: small pills (Autopilot's share, why it always asks, the risk). Under Reduce Motion it appears without
 * sliding.
 */
@Composable
fun AskField(
    field: Field,
    label: String,
    ask: String,
    target: String?,
    onClose: () -> Unit,
    modifier: Modifier = Modifier,
    /** The machine part (a command, a branch, a host) in mono under the ask, whole. */
    details: String? = null,
    /** The request's icon beside the label line (an entry's page). */
    icon: (@Composable () -> Unit)? = null,
    askTag: String = "what",
    closeGlyph: Glyph = Glyph.ArrowBack,
    closeLabel: String = "Back",
    /** The time left, on the left of the close button. */
    timer: (@Composable () -> Unit)? = null,
    /** Who asks, after the request's tile (their mark and name). */
    top: @Composable RowScope.() -> Unit = {},
    verdict: (@Composable () -> Unit)? = null,
    below: (@Composable ColumnScope.() -> Unit)? = null,
    /** Shared by the screen so the field slides in once, not each time a drill-in comes back. */
    entered: MutableTransitionState<Boolean>? = null,
) {
    val c = LocalColors.current
    val reduce = LocalReduceMotion.current
    val own = remember { MutableTransitionState(false).apply { targetState = true } }
    val shown = entered ?: own
    AnimatedVisibility(shown, modifier, enter = Motion.fieldEnter(reduce)) {
        Column(
            Modifier
                .fillMaxWidth()
                .fieldSurface(field, FieldHeaderShape)
                .windowInsetsPadding(HeaderInsets)
                .padding(start = 20.dp, end = 18.dp, top = 2.dp, bottom = 22.dp)
                .testTag("hero"),
        ) {
            // The time left on the left, the close button on the right (Next, Approve 01).
            Row(Modifier.fillMaxWidth().heightIn(min = 36.dp), verticalAlignment = Alignment.CenterVertically) {
                Box(Modifier.weight(1f)) { timer?.invoke() }
                FieldButton(closeGlyph, closeLabel, Modifier.testTag("closeSheet"), size = 32.dp, onClick = onClose)
            }
            // What it touches (its tile, what it does on the corner) and who asks; read out as the label line.
            Row(
                Modifier.padding(top = 14.dp).clearAndSetSemantics {
                    testTag = "fieldLabel"
                    text = androidx.compose.ui.text.AnnotatedString(label)
                },
                verticalAlignment = Alignment.CenterVertically,
                horizontalArrangement = Arrangement.spacedBy(8.dp),
            ) {
                icon?.invoke()
                if (icon != null) RText("·", RType.sans(15f, FontWeight.SemiBold), c.onField.copy(alpha = 0.6f), maxLines = 1)
                top()
            }
            AnimatedVisibility(shown, enter = fadeIn(tween(if (reduce) Motion.REDUCED_MS else 240, delayMillis = if (reduce) 0 else Motion.ASK_DELAY_MS))) {
                Column {
                    // A slash never ends a line ("octo/ app"); what is read out (and tested) is the ask as written.
                    AskText(
                        noBreakAfterSlash(ask),
                        c.onField,
                        Modifier.padding(top = 10.dp).fillMaxWidth().clearAndSetSemantics {
                            testTag = askTag
                            text = androidx.compose.ui.text.AnnotatedString(ask)
                        },
                        target = target?.let(::noBreakAfterSlash),
                        style = RType.ask.copy(lineBreak = androidx.compose.ui.text.style.LineBreak.Heading),
                    )
                    // The machine part in mono, on its own line, whole (ASK-TEXT.md).
                    if (!details.isNullOrBlank()) {
                        RText(details, RType.mono(14f), c.onField.copy(alpha = 0.85f), Modifier.padding(top = 8.dp).testTag("askDetails"), ltr = true)
                    }
                }
            }
            if (verdict != null) Box(Modifier.padding(top = 12.dp)) { verdict() }
            below?.invoke(this)
        }
    }
}

/**
 * Drilled in (Next, Approve 02 and 03): the field shrinks to a band that keeps its colour: back on the left, the page's
 * title over the ask in one line in the middle, the timer on the right, so the ask is never lost.
 */
@Composable
fun AskBand(field: Field, ask: String, target: String?, onBack: () -> Unit, modifier: Modifier = Modifier, title: String? = null, timer: @Composable () -> Unit = {}) {
    val c = LocalColors.current
    Row(
        modifier
            .fillMaxWidth()
            .fieldSurface(field)
            .windowInsetsPadding(HeaderInsets)
            .padding(start = 16.dp, end = 16.dp, top = 6.dp, bottom = 12.dp)
            .testTag("askBand"),
        horizontalArrangement = Arrangement.spacedBy(10.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        FieldButton(Glyph.ArrowBack, "Back", Modifier.testTag("back"), size = 36.dp, onClick = onBack)
        Column(Modifier.weight(1f), horizontalAlignment = Alignment.CenterHorizontally) {
            if (title != null) RText(title, RType.sans(16f, FontWeight.SemiBold), c.onField, Modifier.testTag("pageTitle"), maxLines = 1, ltr = true)
            androidx.compose.foundation.text.BasicText(
                askString(ask, target),
                style = (if (title != null) RType.sans(13f) else RType.sans(15f, FontWeight.Medium)).copy(color = c.onField.copy(alpha = 0.85f), textAlign = androidx.compose.ui.text.style.TextAlign.Center),
                maxLines = 1,
                overflow = TextOverflow.Ellipsis,
            )
        }
        timer()
    }
}

/** A round button on a field: the white wash with a white glyph. */
@Composable
fun FieldButton(glyph: Glyph, label: String, modifier: Modifier = Modifier, size: Dp = 36.dp, onClick: () -> Unit) {
    val c = LocalColors.current
    Box(
        modifier
            .size(size)
            .clip(CircleShape)
            .background(c.onFieldWash)
            .pressable(shape = CircleShape, label = label, onClick = onClick)
            .semantics { contentDescription = label },
        contentAlignment = Alignment.Center,
    ) { GlyphIcon(glyph, c.onField, size = size * 0.5f) }
}

/** Who asks, in the field's top row: their mark on a white disc, the name and where from. */
@Composable
fun RowScope.AskWho(name: String, host: String?, avatar: @Composable () -> Unit) {
    val c = LocalColors.current
    // The mark on its white disc, the same in both themes.
    CompositionLocalProvider(LocalColors provides RColors.Light) { avatar() }
    Row(Modifier.weight(1f), verticalAlignment = Alignment.CenterVertically) {
        RText(name, RType.sans(15f, FontWeight.SemiBold), c.onField, Modifier.weight(1f, fill = false), maxLines = 1)
        if (!host.isNullOrBlank()) RText(" · $host", RType.sans(15f, FontWeight.Medium), c.onField2, maxLines = 1)
    }
}

/**
 * A verdict on a field, as a small pill (`.pill.p-onf`): a glyph (or a ring) and a few words, white on the white wash;
 * a tap opens its detail. [spoken] is what it says to a screen reader when the words are shortened ("97%").
 */
@Composable
fun FieldVerdict(
    text: String,
    modifier: Modifier = Modifier,
    glyph: Glyph? = null,
    ring: Float? = null,
    maxLines: Int = 2,
    chevron: Boolean = false,
    textTag: String? = null,
    spoken: String? = null,
    onClick: (() -> Unit)? = null,
    onOverflow: (Boolean) -> Unit = {},
) {
    val c = LocalColors.current
    val shape = androidx.compose.foundation.shape.RoundedCornerShape(12.dp)
    Row(
        modifier
            .clip(shape)
            .background(c.onField.copy(alpha = 0.2f))
            .then(if (onClick != null) Modifier.pressable(shape = shape, label = spoken ?: text, onClick = onClick) else Modifier)
            .semantics(mergeDescendants = true) { if (spoken != null) contentDescription = spoken }
            .heightIn(min = 24.dp)
            .padding(horizontal = 9.dp, vertical = 3.dp),
        horizontalArrangement = Arrangement.spacedBy(5.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        when {
            ring != null -> dev.reins.android.design.Ring(ring, c.onField, c.onField.copy(alpha = 0.3f), 14.dp)
            glyph != null -> GlyphIcon(glyph, c.onField, size = 13.dp)
        }
        androidx.compose.material3.Text(
            text,
            Modifier.weight(1f, fill = false).then(if (textTag != null) Modifier.testTag(textTag) else Modifier),
            color = c.onField,
            style = RType.sans(12.5f, FontWeight.SemiBold, lineHeight = 16f),
            maxLines = maxLines,
            overflow = TextOverflow.Ellipsis,
            onTextLayout = { onOverflow(it.hasVisualOverflow) },
        )
        if (chevron) GlyphIcon(Glyph.ChevronRight, c.onField, size = 13.dp)
    }
}

/** The decision bar at the bottom of an ask (`.dock`): paper with round top corners and a hairline ring. */
@Composable
fun AskDock(modifier: Modifier = Modifier, content: @Composable ColumnScope.() -> Unit) {
    val c = LocalColors.current
    val shape = androidx.compose.foundation.shape.RoundedCornerShape(topStart = 30.dp, topEnd = 30.dp)
    Column(
        modifier
            .fillMaxWidth()
            .clip(shape)
            .background(c.paper)
            .border(1.dp, c.rule, shape)
            .windowInsetsPadding(WindowInsets.navigationBars)
            .padding(start = 14.dp, end = 14.dp, top = 12.dp, bottom = 16.dp)
            .testTag("dock"),
        verticalArrangement = Arrangement.spacedBy(10.dp),
        content = content,
    )
}

/** Deny (outline) and the action (inverse, with the fingerprint), for an ask without a chooser (pair, join, a file). */
@Composable
fun AskButtons(
    action: String,
    onDeny: () -> Unit,
    onApprove: () -> Unit,
    busy: Boolean,
    actionEnabled: Boolean = true,
    deny: String = "Deny",
) {
    val c = LocalColors.current
    Row(Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.spacedBy(10.dp)) {
        dev.reins.android.design.SignalButton(deny, Modifier.weight(1f).testTag("deny"), style = dev.reins.android.design.ButtonStyle.Outline, enabled = !busy, onClick = onDeny)
        dev.reins.android.design.SignalButton(
            action,
            Modifier.weight(1.35f).testTag("approve"),
            enabled = !busy && actionEnabled,
            leading = { if (busy) dev.reins.android.design.Spinner(c.onInverse, size = 18.dp) else GlyphIcon(Glyph.Fingerprint, c.onInverse, size = 20.dp) },
            onClick = onApprove,
        )
    }
}

/** An ask that is still loading, or failed to load: a close button on paper, the spinner or what went wrong. */
@Composable
fun AskLoading(loading: Boolean, error: String?, onClose: () -> Unit, errorTag: String? = null) {
    val c = LocalColors.current
    Column(Modifier.fillMaxWidth().windowInsetsPadding(HeaderInsets).padding(start = 18.dp, end = 18.dp, top = 4.dp)) {
        dev.reins.android.design.RoundIconButton(Glyph.Close, "Close", Modifier.testTag("closeSheet"), onClick = onClose)
        Column(Modifier.fillMaxWidth().padding(32.dp), horizontalAlignment = Alignment.CenterHorizontally) {
            if (loading) dev.reins.android.design.Spinner(c.inkMeta, size = 32.dp)
            error?.let { dev.reins.android.design.Banner(it, Modifier.padding(top = 24.dp), dev.reins.android.design.BannerKind.Error, tag = errorTag) }
        }
    }
}

/** [text] with a word joiner after each slash, so a path or a repo ("octo/app") is never split at its slash. */
internal fun noBreakAfterSlash(text: String): String = text.replace("/", "/⁠")

/** A timer on a field: the ring draining each second, a white pill with red digits in the last 15 s. Its own clock. */
@Composable
fun FieldTimer(createdAt: Long, waitUntil: Long?) {
    if (waitUntil == null) return
    val now = dev.reins.android.design.rememberNowState(1_000, untilMillis = waitUntil * 1000)
    val left = (waitUntil - now.value / 1000).coerceAtLeast(0)
    Box(Modifier.testTag("waitLine")) { dev.reins.android.design.TimerPill(left, (waitUntil - createdAt).coerceAtLeast(1)) }
}
