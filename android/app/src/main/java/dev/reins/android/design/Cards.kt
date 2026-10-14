package dev.reins.android.design

import androidx.compose.foundation.Canvas
import androidx.compose.foundation.Image
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.ColumnScope
import androidx.compose.foundation.layout.WindowInsets
import androidx.compose.foundation.layout.defaultMinSize
import androidx.compose.foundation.layout.widthIn
import androidx.compose.ui.draw.rotate
import androidx.compose.ui.semantics.semantics
import androidx.compose.material.icons.rounded.ChevronRight
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.navigationBars
import androidx.compose.foundation.layout.offset
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.windowInsetsPadding
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.Icon
import androidx.compose.runtime.Composable
import androidx.compose.runtime.remember
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Brush
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.ColorFilter
import androidx.compose.ui.graphics.StrokeCap
import androidx.compose.ui.graphics.asImageBitmap
import androidx.compose.ui.graphics.drawscope.Stroke
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp

// The pieces of the Activity and Grants cards: a ring, a brand tile with the AI's face on its corner, a tinted card,
// and the dock a page's own actions are pinned in.

/** A progress ring: [fraction] of the circle in [color] from 12 o'clock, clockwise, over a [track]. */
@Composable
fun ProgressRing(fraction: Float, color: Color, track: Color, size: Dp, modifier: Modifier = Modifier, stroke: Dp = size * 0.125f) {
    Canvas(modifier.size(size)) {
        val w = stroke.toPx()
        val inset = w / 2
        val arc = androidx.compose.ui.geometry.Size(this.size.width - w, this.size.height - w)
        val topLeft = androidx.compose.ui.geometry.Offset(inset, inset)
        drawArc(track, 0f, 360f, false, topLeft, arc, style = Stroke(w))
        val f = fraction.coerceIn(0f, 1f)
        if (f > 0f) drawArc(color, -90f, 360f * f, false, topLeft, arc, style = Stroke(w, cap = StrokeCap.Round))
    }
}

/** The plate of a brand tile: near-black with a hairline in the dark, white in the light. */
fun tilePlate(c: RColors): Color = if (c.dark) Color(0xFF1C1C20) else Color.White

fun tileEdge(c: RColors): Color = if (c.dark) Color(0xFF2C2C31) else c.hairline

/** A service's logo on a rounded square ([ServiceAvatar] is the round one). Without a logo, [fallback] on the tile. */
@Composable
fun ServiceTile(service: String, modifier: Modifier = Modifier, size: Dp = 44.dp, fallback: ImageVector? = null, fallbackTint: Color? = null) {
    val c = LocalColors.current
    val context = LocalContext.current
    val markSize = size * 0.5f
    val px = with(LocalDensity.current) { markSize.roundToPx() }
    val mark = remember(service, px) { if (service.isBlank()) null else SvgAssets.bitmap(context, "services/$service.svg", px)?.asImageBitmap() }
    val shape = RoundedCornerShape(size * 0.32f)
    Box(
        modifier.size(size).clip(shape).background(tilePlate(c)).border(1.dp, tileEdge(c), shape),
        contentAlignment = Alignment.Center,
    ) {
        when {
            mark != null -> Image(mark, service, Modifier.size(markSize), colorFilter = ColorFilter.tint(serviceTint(service, c)))
            fallback != null -> Icon(fallback, null, Modifier.size(size * 0.5f), tint = fallbackTint ?: c.secondary)
        }
    }
}

/** A brand's colour on the tiles: Gmail's red a touch softer in the dark, like the mockups. */
fun serviceTint(service: String, c: RColors): Color = when (service) {
    "gmail" -> if (c.dark) Color(0xFFF87171) else Color(0xFFEA4335)
    "gcalendar" -> if (c.dark) Color(0xFF60A5FA) else Color(0xFF4285F4)
    else -> serviceColor(service, c)
}

/**
 * The "mini": what is asked of (a service's tile) with who asks (the AI's face) on its bottom-left corner, ringed in
 * the colour of what is behind it so the two read as one.
 */
@Composable
fun MiniLogo(
    service: String,
    face: @Composable (Dp) -> Unit,
    modifier: Modifier = Modifier,
    size: Dp = 44.dp,
    faceSize: Dp = 22.dp,
    ring: Color = LocalColors.current.background,
    fallback: ImageVector? = null,
    fallbackTint: Color? = null,
) {
    Box(modifier.size(size)) {
        ServiceTile(service, size = size, fallback = fallback, fallbackTint = fallbackTint)
        Box(
            Modifier
                .align(Alignment.BottomStart)
                .offset(x = -(faceSize * 0.27f), y = faceSize * 0.27f)
                .size(faceSize + 5.dp)
                .background(ring, CircleShape),
            contentAlignment = Alignment.Center,
        ) { face(faceSize) }
    }
}

/** A card washed in [tone] like the approval hero: a gradient from 16 % to 4 % and a 24 % edge. */
fun Modifier.toneCard(tone: Color, radius: Dp = 24.dp, top: Float = 0.16f, bottom: Float = 0.04f, edge: Float = 0.24f): Modifier {
    val shape = RoundedCornerShape(radius)
    return this
        .clip(shape)
        .background(Brush.verticalGradient(listOf(tone.copy(alpha = top), tone.copy(alpha = bottom))), shape)
        .border(1.dp, tone.copy(alpha = edge), shape)
}

/** A plain card of the lists (a grant, the rows of a group). */
fun Modifier.plainCard(c: RColors, radius: Dp = 22.dp): Modifier {
    val shape = RoundedCornerShape(radius)
    return this
        .clip(shape)
        .background(if (c.dark) Color(0xFF111113) else c.elevated, shape)
        .border(1.dp, if (c.dark) Color(0xFF1C1C20) else c.hairline, shape)
}

/**
 * A page's own actions, pinned at the bottom like the approval dock: a raised plate with round top corners, over the
 * navigation bar.
 */
@Composable
fun PageDock(modifier: Modifier = Modifier, content: @Composable ColumnScope.() -> Unit) {
    val c = LocalColors.current
    val shape = RoundedCornerShape(topStart = 28.dp, topEnd = 28.dp)
    Column(
        modifier
            .fillMaxWidth()
            .clip(shape)
            .background(if (c.dark) Color(0xFF101012) else c.elevated, shape)
            .border(1.dp, if (c.dark) Color(0xFF1E1E22) else c.hairline, shape)
            .windowInsetsPadding(WindowInsets.navigationBars)
            .padding(start = 14.dp, end = 14.dp, top = 14.dp, bottom = 18.dp),
        content = content,
    )
}

/** The dark text on the warning (yellow) colour. */
val OnWarning = Color(0xFF231A00)

/** The dark text on the success (green) colour in the dark theme; white on the light theme's deeper green. */
fun onSuccess(c: RColors): Color = if (c.dark) Color(0xFF04210F) else Color.White

/**
 * A row of a detail page's card: a quiet label and its value ("When  Today, 14:14"), or, when it opens something, the
 * label in the text colour with a count and a chevron ("Emails shared  2 ›"). [open] turns the chevron down.
 */
@Composable
fun ValueRow(
    label: String,
    modifier: Modifier = Modifier,
    value: String? = null,
    mono: Boolean = false,
    open: Boolean? = null,
    valueColor: Color? = null,
    onClick: (() -> Unit)? = null,
) {
    val c = LocalColors.current
    androidx.compose.foundation.layout.Row(
        modifier
            .fillMaxWidth()
            .defaultMinSize(minHeight = 56.dp)
            .then(if (onClick != null) Modifier.pressable(highlight = c.controlFill, shape = RoundedCornerShape(0.dp), onClick = onClick) else Modifier)
            .semantics(mergeDescendants = true) {}
            .padding(horizontal = 16.dp, vertical = 12.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        RText(
            label,
            RType.sans(16f, if (onClick != null) androidx.compose.ui.text.font.FontWeight.Medium else androidx.compose.ui.text.font.FontWeight.Normal),
            if (onClick != null) c.text else c.tertiary,
            Modifier.weight(1f),
            maxLines = 1,
        )
        if (value != null) {
            androidx.compose.foundation.layout.Spacer(Modifier.size(width = 12.dp, height = 1.dp))
            RText(
                value,
                if (mono) RType.mono(14f) else RType.sans(15f),
                valueColor ?: if (onClick != null) c.secondary else c.text,
                Modifier.widthIn(max = 230.dp),
                maxLines = 1,
                ltr = mono,
            )
        }
        if (onClick != null) {
            androidx.compose.foundation.layout.Spacer(Modifier.size(width = 6.dp, height = 1.dp))
            Icon(
                androidx.compose.material.icons.Icons.Rounded.ChevronRight,
                null,
                Modifier.size(20.dp).rotate(if (open == true) 90f else 0f),
                tint = c.tertiary,
            )
        }
    }
}
