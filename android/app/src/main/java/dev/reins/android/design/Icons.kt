package dev.reins.android.design

import androidx.compose.foundation.Canvas
import androidx.compose.foundation.layout.size
import androidx.compose.runtime.Composable
import androidx.compose.runtime.Immutable
import androidx.compose.ui.Modifier
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.Matrix
import androidx.compose.ui.graphics.Path
import androidx.compose.ui.graphics.SolidColor
import androidx.compose.ui.graphics.drawscope.Fill
import androidx.compose.ui.graphics.drawscope.scale
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.graphics.vector.PathParser
import androidx.compose.ui.graphics.vector.addPathNodes
import androidx.compose.ui.graphics.vector.group
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp

/** One filled shape of a [Glyph]: Material Symbols path data on the symbols' 960 grid (y from -960 to 0). */
@Immutable
class GlyphPart(val d: String) {
    /** Parsed once, on first draw, and moved onto the 24 grid. */
    val path: Path by lazy {
        PathParser().parsePathString(d).toPath().apply {
            transform(Matrix().apply { scale(GRID / SYMBOL_GRID, GRID / SYMBOL_GRID); translate(0f, SYMBOL_GRID) })
        }
    }

    private companion object {
        const val GRID = 24f
        const val SYMBOL_GRID = 960f
    }
}

/**
 * A glyph drawn in [tint] at [size]. [weight] is kept for callers from before Next; Material Symbols carry their own
 * weight (400), so it changes nothing.
 */
@Composable
@Suppress("UNUSED_PARAMETER")
fun GlyphIcon(glyph: Glyph, tint: Color, size: Dp = 18.dp, modifier: Modifier = Modifier, weight: Float = 1.6f) {
    Canvas(modifier.size(size)) {
        val k = this.size.minDimension / 24f
        scale(k, k, pivot = Offset.Zero) {
            for (part in glyph.parts) drawPath(part.path, tint, style = Fill)
        }
    }
}

/** [glyph] as an image vector for `Icon` and friends: black, to be tinted. */
internal fun glyphVector(glyph: Glyph): ImageVector = ImageVector.Builder(
    name = "Glyph.${glyph.name}",
    defaultWidth = 24.dp,
    defaultHeight = 24.dp,
    viewportWidth = 960f,
    viewportHeight = 960f,
).apply {
    group(translationY = 960f) {
        for (part in glyph.parts) addPath(pathData = addPathNodes(part.d), fill = SolidColor(Color.Black))
    }
}.build()
