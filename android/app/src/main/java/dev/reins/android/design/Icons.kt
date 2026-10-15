package dev.reins.android.design

import androidx.compose.foundation.Canvas
import androidx.compose.foundation.layout.size
import androidx.compose.runtime.Composable
import androidx.compose.runtime.Immutable
import androidx.compose.ui.Modifier
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.Path
import androidx.compose.ui.graphics.PathFillType
import androidx.compose.ui.graphics.SolidColor
import androidx.compose.ui.graphics.StrokeCap
import androidx.compose.ui.graphics.StrokeJoin
import androidx.compose.ui.graphics.drawscope.Fill
import androidx.compose.ui.graphics.drawscope.Stroke
import androidx.compose.ui.graphics.drawscope.scale
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.graphics.vector.PathParser
import androidx.compose.ui.graphics.vector.addPathNodes
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp

/**
 * One shape of a [Glyph], on the 24-unit grid: stroked in the tint (1.6 units, round caps and joins) unless [stroke]
 * is false, filled in the tint when [fill], white instead of the tint when [white].
 */
@Immutable
class GlyphPart(
    val d: String,
    val fill: Boolean = false,
    val stroke: Boolean = true,
    val white: Boolean = false,
    val width: Float = 1.6f,
    /** Filled even-odd: inner shapes knock out (the tick in a filled shield). */
    val evenOdd: Boolean = false,
) {
    /** Parsed once, on first draw. */
    val path: Path by lazy { PathParser().parsePathString(d).toPath(Path().apply { if (evenOdd) fillType = PathFillType.EvenOdd }) }
}

/** A glyph drawn in [tint] at [size]; [weight] is the stroke on the 24 grid (1.6, the set's one weight). */
@Composable
fun GlyphIcon(glyph: Glyph, tint: Color, size: Dp = 18.dp, modifier: Modifier = Modifier, weight: Float = 1.6f) {
    Canvas(modifier.size(size)) {
        val k = this.size.minDimension / 24f
        scale(k, k, pivot = Offset.Zero) {
            for (part in glyph.parts) {
                val color = if (part.white) Color.White else tint
                if (part.fill) drawPath(part.path, color, style = Fill)
                if (part.stroke) {
                    val w = part.width * weight / 1.6f
                    drawPath(part.path, color, style = Stroke(width = w, cap = StrokeCap.Round, join = StrokeJoin.Round))
                }
            }
        }
    }
}

/** [glyph] as an image vector for `Icon` and friends: black, to be tinted. */
internal fun glyphVector(glyph: Glyph): ImageVector = ImageVector.Builder(
    name = "Glyph.${glyph.name}",
    defaultWidth = 24.dp,
    defaultHeight = 24.dp,
    viewportWidth = 24f,
    viewportHeight = 24f,
).apply {
    for (part in glyph.parts) {
        val paint = SolidColor(if (part.white) Color.White else Color.Black)
        addPath(
            pathData = addPathNodes(part.d),
            pathFillType = if (part.evenOdd) PathFillType.EvenOdd else PathFillType.NonZero,
            fill = if (part.fill) paint else null,
            stroke = if (part.stroke) paint else null,
            strokeLineWidth = part.width,
            strokeLineCap = StrokeCap.Round,
            strokeLineJoin = StrokeJoin.Round,
        )
    }
}.build()
