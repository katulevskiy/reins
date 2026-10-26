package dev.reins.android.design

import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.ColumnScope
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.RowScope
import androidx.compose.foundation.layout.defaultMinSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.draw.drawBehind
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.heading
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp

// The Next page kit (System board, "Page templates"): Standard pages put rows in raised groups under a section header;
// Quiet pages (Settings and below) put them flat between hairlines. Both share the row.

/**
 * A section header (`.sec`): an optional glyph, the words (13 / 600, ink-2), and at the end a count and an optional
 * round + that adds to the section.
 */
@Composable
fun SectionHeader(
    text: String,
    modifier: Modifier = Modifier,
    glyph: Glyph? = null,
    count: String? = null,
    quiet: Boolean = false,
    onAdd: (() -> Unit)? = null,
    addLabel: String = "Add",
    addTag: String? = null,
    trailing: (@Composable RowScope.() -> Unit)? = null,
    /** The words' and glyph's colour, when not ink-2 (a verdict's). */
    ink: Color? = null,
    /** The count's colour (a running mode's). */
    countInk: Color? = null,
) {
    val c = LocalColors.current
    val ink = ink ?: if (quiet) c.inkMeta else c.ink2
    Row(
        modifier
            .fillMaxWidth()
            .padding(start = if (quiet) 20.dp else 24.dp, end = if (onAdd != null) 16.dp else 24.dp, top = if (quiet) 20.dp else 22.dp, bottom = if (quiet) 6.dp else 8.dp)
            .semantics(mergeDescendants = true) { heading() },
        horizontalArrangement = Arrangement.spacedBy(6.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        glyph?.let { GlyphIcon(it, ink, size = 14.dp) }
        RText(text, if (quiet) RType.sans(12.5f, FontWeight.SemiBold, tracking = 0.015f) else RType.section, ink, Modifier.weight(1f), maxLines = 1)
        trailing?.invoke(this)
        count?.let { RText(it, RType.section, countInk ?: ink, maxLines = 1) }
        if (onAdd != null) {
            Box(
                Modifier
                    .padding(start = 6.dp)
                    .size(32.dp)
                    .clip(CircleShape)
                    .background(c.paperRaised)
                    .border(1.dp, c.rule, CircleShape)
                    .pressable(shape = CircleShape, label = addLabel, onClick = onAdd)
                    .semantics { contentDescription = addLabel }
                    .then(if (addTag != null) Modifier.testTag(addTag) else Modifier),
                contentAlignment = Alignment.Center,
            ) { GlyphIcon(Glyph.Plus, c.ink, size = 18.dp) }
        }
    }
}

/** A raised group (`.grp`): 16 in from the edges, radius 20, a hairline ring in light; frosted over a wallpaper. */
@Composable
fun PaneGroup(modifier: Modifier = Modifier, content: @Composable ColumnScope.() -> Unit) {
    Column(modifier.padding(horizontal = 16.dp).fillMaxWidth().pane(), content = content)
}

/** Flat rows (`.flat`): 20 in from the edges, a hairline above and below; the rows draw their own between. */
@Composable
fun FlatGroup(modifier: Modifier = Modifier, content: @Composable ColumnScope.() -> Unit) {
    val c = LocalColors.current
    Column(
        modifier
            .padding(horizontal = 20.dp)
            .fillMaxWidth()
            .drawBehind {
                drawLine(c.rule, Offset(0f, 0.5.dp.toPx()), Offset(size.width, 0.5.dp.toPx()), strokeWidth = 1.dp.toPx())
                drawLine(c.rule, Offset(0f, size.height - 0.5.dp.toPx()), Offset(size.width, size.height - 0.5.dp.toPx()), strokeWidth = 1.dp.toPx())
            },
        content = content,
    )
}

/** A small tile for a row (`.tile`): 32 dp, radius 9, the glyph in ink-2 on sunken paper (or [tint] on its wash). */
@Composable
fun RowTile(glyph: Glyph, modifier: Modifier = Modifier, tint: Color? = null, fill: Color? = null, size: Dp = 32.dp) {
    val c = LocalColors.current
    Box(
        modifier.size(size).clip(RoundedCornerShape(size * 0.28f)).background(fill ?: tint?.copy(alpha = 0.13f) ?: c.paperSunken),
        contentAlignment = Alignment.Center,
    ) { GlyphIcon(glyph, if (fill != null) Color.White else tint ?: c.ink2, size = size * 0.56f) }
}

/**
 * A row of a group (`.r`): optional [leading] (a [RowTile], an avatar), the title (16 / 500) over an optional
 * [subtitle], then [value] (15, ink-2), [trailing] and a chevron. [first] rows draw no hairline above; the hairline
 * starts past the leading slot ([inset]) and, in a raised group, stops 0 from the edge (16 in a frosted one).
 */
@Composable
fun PaneRow(
    title: String,
    modifier: Modifier = Modifier,
    subtitle: String? = null,
    leading: (@Composable () -> Unit)? = null,
    value: String? = null,
    valueMono: Boolean = false,
    trailing: (@Composable () -> Unit)? = null,
    chevron: Boolean = false,
    first: Boolean = false,
    flat: Boolean = false,
    titleColor: Color? = null,
    titleMono: Boolean = false,
    tag: String? = null,
    enabled: Boolean = true,
    /** Another's words before the title (an asker's name), in ink-2 so they never pass for Reins' own. */
    titleLead: String? = null,
    onClick: (() -> Unit)? = null,
) {
    val c = LocalColors.current
    val inset = when {
        flat && leading != null -> 44.dp
        flat -> 0.dp
        leading != null -> 62.dp
        else -> 14.dp
    }
    Row(
        modifier
            .fillMaxWidth()
            .then(if (!first) Modifier.drawBehind { drawLine(c.rule, Offset(inset.toPx(), 0.5.dp.toPx()), Offset(size.width, 0.5.dp.toPx()), strokeWidth = 1.dp.toPx()) } else Modifier)
            .then(if (onClick != null) Modifier.pressable(enabled = enabled, highlight = c.paperSunken, shape = RoundedCornerShape(0.dp), onClick = onClick) else Modifier)
            .then(if (tag != null) Modifier.testTag(tag) else Modifier)
            .semantics(mergeDescendants = true) {}
            .defaultMinSize(minHeight = 56.dp)
            .padding(horizontal = if (flat) 0.dp else 14.dp, vertical = 9.dp),
        horizontalArrangement = Arrangement.spacedBy(12.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        leading?.invoke()
        Column(Modifier.weight(1f)) {
            if (titleLead.isNullOrEmpty()) {
                RText(title, if (titleMono) RType.mono(14f) else RType.row, titleColor ?: c.ink, maxLines = 2, ltr = titleMono)
            } else {
                androidx.compose.foundation.text.BasicText(
                    androidx.compose.ui.text.buildAnnotatedString {
                        pushStyle(androidx.compose.ui.text.SpanStyle(color = c.ink2))
                        append(titleLead)
                        pop()
                        append(" ")
                        append(title)
                    },
                    style = RType.row.copy(color = titleColor ?: c.ink),
                    maxLines = 2,
                    overflow = androidx.compose.ui.text.style.TextOverflow.Ellipsis,
                )
            }
            if (!subtitle.isNullOrEmpty()) RText(subtitle, RType.sans(13f, lineHeight = 17f), c.inkMeta, Modifier.padding(top = 1.dp), maxLines = 3)
        }
        value?.let { RText(it, if (valueMono) RType.mono(13f) else RType.sans(15f), c.ink2, maxLines = 1, ltr = valueMono) }
        trailing?.invoke()
        if (chevron) GlyphIcon(Glyph.ChevronRight, c.inkFaint, size = 18.dp)
    }
}

/** A small tag (`.pill`): 24 dp, 12.5 / 600, [tint] on its 13 % wash; [neutral] is ink-2 on sunken paper. */
@Composable
fun NextPill(text: String, tint: Color?, modifier: Modifier = Modifier, glyph: Glyph? = null, textTag: String? = null) {
    val c = LocalColors.current
    val ink = tint ?: c.ink2
    Row(
        modifier.heightIn(min = 24.dp).clip(CircleShape).background(tint?.copy(alpha = 0.13f) ?: c.paperSunken).padding(horizontal = 9.dp),
        horizontalArrangement = Arrangement.spacedBy(4.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        glyph?.let { GlyphIcon(it, ink, size = 12.dp) }
        RText(text, RType.sans(12.5f, FontWeight.SemiBold), ink, if (textTag != null) Modifier.testTag(textTag) else Modifier, maxLines = 1)
    }
}
