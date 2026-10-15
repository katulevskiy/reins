package dev.reins.android.design

import android.content.Context
import android.graphics.BitmapFactory
import android.graphics.BitmapShader
import android.graphics.Matrix
import android.graphics.Shader
import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import androidx.compose.ui.geometry.Size
import androidx.compose.ui.graphics.BlendMode
import androidx.compose.ui.graphics.Outline
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.RectangleShape
import androidx.compose.ui.graphics.ShaderBrush
import androidx.compose.ui.graphics.Shape
import androidx.compose.ui.graphics.drawOutline
import androidx.compose.ui.graphics.drawscope.ContentDrawScope
import androidx.compose.ui.node.DrawModifierNode
import androidx.compose.ui.node.invalidateDraw
import androidx.compose.ui.node.ModifierNodeElement
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.unit.LayoutDirection
import androidx.compose.ui.unit.dp
import dev.reins.android.R

/**
 * The grain on colour fields: one tileable grey noise (`res/drawable-nodpi/grain.png`, 360 px) shown one tile per
 * 180 dp, blended soft-light over the field. Only on fields; never on paper, text or glyphs; never animated.
 */
object Grain {
    /** One tile of the texture covers this much of the field. */
    val TILE = 180.dp

    @Volatile private var bitmap: android.graphics.Bitmap? = null

    /** The texture, decoded once per process. */
    fun bitmap(context: Context): android.graphics.Bitmap = bitmap ?: synchronized(this) {
        bitmap ?: BitmapFactory.decodeResource(context.resources, R.drawable.grain, BitmapFactory.Options().apply { inScaled = false })
            .also { bitmap = it }
    }
}

/**
 * A colour field: [color] filling [shape], with the grain over it (soft light) unless [LocalSignalPrefs] turned it
 * off (Appearance, or the system's high contrast). The content draws above both.
 */
@Composable
fun Modifier.fieldSurface(color: Color, shape: Shape = RectangleShape): Modifier {
    val texture = if (LocalSignalPrefs.current.grain) Grain.bitmap(LocalContext.current) else null
    return this then FieldSurfaceElement(color, shape, texture)
}

private data class FieldSurfaceElement(val color: Color, val shape: Shape, val texture: android.graphics.Bitmap?) :
    ModifierNodeElement<FieldSurfaceNode>() {
    override fun create() = FieldSurfaceNode(color, shape, texture)

    override fun update(node: FieldSurfaceNode) {
        node.update(color, shape, texture)
    }
}

/**
 * Draws the field behind its content. The outline and the grain's shader are made once per size (and density),
 * not per frame: a field whose content redraws every second draws from the cache.
 */
private class FieldSurfaceNode(
    private var color: Color,
    private var shape: Shape,
    private var texture: android.graphics.Bitmap?,
) : Modifier.Node(), DrawModifierNode {
    private var cachedSize = Size.Unspecified
    private var cachedDirection: LayoutDirection? = null
    private var cachedDensity = 0f
    private var outline: Outline? = null
    private var brush: ShaderBrush? = null

    fun update(color: Color, shape: Shape, texture: android.graphics.Bitmap?) {
        if (shape != this.shape || texture !== this.texture) {
            outline = null
            brush = null
        }
        this.color = color
        this.shape = shape
        this.texture = texture
        invalidateDraw()
    }

    override fun ContentDrawScope.draw() {
        if (outline == null || size != cachedSize || layoutDirection != cachedDirection || density != cachedDensity) {
            cachedSize = size
            cachedDirection = layoutDirection
            cachedDensity = density
            outline = shape.createOutline(size, layoutDirection, this)
            brush = texture?.let { bmp ->
                val scale = Grain.TILE.toPx() / bmp.width
                ShaderBrush(
                    BitmapShader(bmp, Shader.TileMode.REPEAT, Shader.TileMode.REPEAT).apply {
                        setLocalMatrix(Matrix().apply { setScale(scale, scale) })
                    },
                )
            }
        }
        val o = outline!!
        drawOutline(o, color)
        brush?.let { drawOutline(o, it, blendMode = BlendMode.Softlight) }
        drawContent()
    }
}

/** A [Field] as a surface: its solid colour, with the grain. */
@Composable
fun Modifier.fieldSurface(field: Field, shape: Shape = RectangleShape): Modifier = fieldSurface(field.fill(), shape)
