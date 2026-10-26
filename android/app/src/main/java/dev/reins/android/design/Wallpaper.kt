package dev.reins.android.design

import android.content.Context
import android.graphics.Bitmap
import android.graphics.BitmapFactory
import android.graphics.Canvas as AndroidCanvas
import android.graphics.ColorMatrix
import android.graphics.ColorMatrixColorFilter
import android.graphics.Paint
import androidx.compose.foundation.Image
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.runtime.Composable
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.runtime.Stable
import androidx.compose.runtime.compositionLocalOf
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.draw.drawBehind
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.geometry.Rect
import androidx.compose.ui.geometry.Size
import androidx.compose.ui.graphics.Brush
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.ImageBitmap
import androidx.compose.ui.graphics.Shape
import androidx.compose.ui.graphics.asImageBitmap
import androidx.compose.ui.graphics.drawOutline
import androidx.compose.ui.graphics.drawscope.clipPath
import androidx.compose.ui.graphics.drawscope.clipRect
import androidx.compose.ui.graphics.drawscope.translate
import androidx.compose.ui.layout.ContentScale
import androidx.compose.ui.layout.boundsInRoot
import androidx.compose.ui.layout.onGloballyPositioned
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.IntOffset
import androidx.compose.ui.unit.IntSize
import androidx.compose.ui.unit.dp
import java.io.File

/**
 * The art behind Activity (Reins Next, "Activity with a wallpaper"): dithered images that fade into the paper at their
 * foot. Halo is the dark default, Torii dither the light one; any of them (or the owner's own photo, made on the
 * Wallpaper page) can be picked for either theme.
 */
enum class Wallpaper(val label: String, private val asset: String?) {
    Halo("Halo", "halo.jpg"),
    Crescent("Crescent", "crescent.jpg"),
    Figure("Figure", "figure.jpg"),
    Hands("Hands", "hands.jpg"),
    Ruins("Ruins", "ruins.jpg"),
    Eyes("Eyes", "eyes.jpg"),
    Torii("Torii", "torii.jpg"),
    ToriiDither("Torii dither", "torii-dither.jpg"),

    /** The owner's own photo, with its effect and fade baked in (one file per theme). */
    Photo("Your photo", null),
    ;

    /** The image, decoded once per process (the owner's photo is read again after it changes). */
    fun bitmap(context: Context, dark: Boolean): Bitmap? = Wallpapers.bitmap(context, this, dark)

    internal fun assetPath(): String? = asset?.let { "wallpapers/$it" }

    companion object {
        fun named(name: String?): Wallpaper? = entries.firstOrNull { it.name == name }

        /** The shipped images, in the picker's order. */
        val shipped: List<Wallpaper> get() = entries.filter { it != Photo }
    }
}

/** Decoding and the frosted copies, cached per process. */
object Wallpapers {
    private val cache = HashMap<String, Bitmap>()
    private val frosted = HashMap<String, Bitmap>()

    /** Bumped when the owner's photo changes, so screens read it again. */
    var photoVersion by mutableStateOf(0)

    /** Where the owner's photo for a theme lives (already processed, JPEG). */
    fun photoFile(context: Context, dark: Boolean): File = File(context.filesDir, if (dark) "wallpaper-dark.jpg" else "wallpaper-light.jpg")

    fun photoChanged() {
        synchronized(this) {
            cache.keys.filter { it.startsWith("photo:") }.forEach { cache.remove(it) }
            frosted.keys.filter { it.startsWith("photo:") }.forEach { frosted.remove(it) }
        }
        photoVersion++
    }

    private fun key(w: Wallpaper, dark: Boolean) = if (w == Wallpaper.Photo) "photo:$dark" else w.name

    fun bitmap(context: Context, w: Wallpaper, dark: Boolean): Bitmap? = synchronized(this) {
        val k = key(w, dark)
        cache[k] ?: run {
            val decoded = runCatching {
                val path = w.assetPath()
                if (path != null) {
                    context.assets.open(path).use { BitmapFactory.decodeStream(it) }
                } else {
                    photoFile(context, dark).takeIf { it.isFile }?.let { BitmapFactory.decodeFile(it.absolutePath) }
                }
            }.getOrNull()
            decoded?.also { cache[k] = it }
        }
    }

    /**
     * The image as a pane sees it through the glass: a small copy, box-blurred, saturated 1.9× and brightened 1.06
     * (light) or darkened to 0.82 (dark): the boards' `backdrop-filter: blur(40px) saturate(1.9) brightness(…)`.
     * Drawn back at full size it is soft; the fine detail that fights text is gone.
     */
    fun frosted(context: Context, w: Wallpaper, dark: Boolean): Bitmap? {
        val k = key(w, dark) + ":" + dark
        synchronized(this) { frosted[k]?.let { return it } }
        val src = bitmap(context, w, dark) ?: return null
        val small = Bitmap.createScaledBitmap(src, (src.width / 12).coerceAtLeast(4), (src.height / 12).coerceAtLeast(4), true)
        val out = Bitmap.createBitmap(small.width, small.height, Bitmap.Config.ARGB_8888)
        val matrix = ColorMatrix().apply {
            setSaturation(1.9f)
            val b = if (dark) 0.82f else 1.06f
            postConcat(ColorMatrix(floatArrayOf(b, 0f, 0f, 0f, 0f, 0f, b, 0f, 0f, 0f, 0f, 0f, b, 0f, 0f, 0f, 0f, 0f, 1f, 0f)))
        }
        AndroidCanvas(out).drawBitmap(small, 0f, 0f, Paint(Paint.FILTER_BITMAP_FLAG).apply { colorFilter = ColorMatrixColorFilter(matrix) })
        boxBlur(out, 2)
        boxBlur(out, 2)
        synchronized(this) { frosted[k] = out }
        return out
    }

    /** One horizontal and one vertical box-blur pass of [radius] pixels, in place. */
    private fun boxBlur(bmp: Bitmap, radius: Int) {
        val w = bmp.width
        val h = bmp.height
        val px = IntArray(w * h)
        bmp.getPixels(px, 0, w, 0, 0, w, h)
        val tmp = IntArray(w * h)
        fun pass(src: IntArray, dst: IntArray, horizontal: Boolean) {
            val lines = if (horizontal) h else w
            val len = if (horizontal) w else h
            for (line in 0 until lines) {
                for (i in 0 until len) {
                    var r = 0
                    var g = 0
                    var b = 0
                    var n = 0
                    for (d in -radius..radius) {
                        val j = (i + d).coerceIn(0, len - 1)
                        val p = if (horizontal) src[line * w + j] else src[j * w + line]
                        r += (p shr 16) and 0xFF
                        g += (p shr 8) and 0xFF
                        b += p and 0xFF
                        n++
                    }
                    val at = if (horizontal) line * w + i else i * w + line
                    dst[at] = (0xFF shl 24) or ((r / n) shl 16) or ((g / n) shl 8) or (b / n)
                }
            }
        }
        pass(px, tmp, true)
        pass(tmp, px, false)
        bmp.setPixels(px, 0, w, 0, 0, w, h)
    }
}

/** Where the art is drawn on screen, for panes to look through. Null when the page has no art. */
@Stable
class Backdrop(val image: ImageBitmap, val frosted: ImageBitmap?) {
    /** The art's bounds in the root, updated as the page lays out. */
    var bounds by mutableStateOf(Rect.Zero)
}

val LocalBackdrop = compositionLocalOf<Backdrop?> { null }

/** The art's height on a page: the boards' 500 of an 844-tall phone. */
val WALLPAPER_HEIGHT: Dp = 500.dp

/**
 * A page with the chosen wallpaper behind it: the art at the top, full width, [WALLPAPER_HEIGHT] tall, cropped from its
 * top, with a scrim under the status bar and title (paper at 75 % in light, black at 55 % in dark, fading out over
 * 150 dp). Panes inside ([Modifier.pane]) frost the art. The art stays when panes turn solid.
 */
@Composable
fun WallpaperPage(wallpaper: Wallpaper?, modifier: Modifier = Modifier, content: @Composable () -> Unit) {
    val c = LocalColors.current
    val context = LocalContext.current
    val version = Wallpapers.photoVersion
    val backdrop = remember(wallpaper, c.dark, version) {
        val bmp = wallpaper?.bitmap(context, c.dark) ?: return@remember null
        Backdrop(bmp.asImageBitmap(), Wallpapers.frosted(context, wallpaper, c.dark)?.asImageBitmap())
    }
    Box(modifier.background(c.paper)) {
        if (backdrop != null) {
            Box(
                Modifier
                    .fillMaxWidth()
                    .height(WALLPAPER_HEIGHT)
                    .onGloballyPositioned { backdrop.bounds = it.boundsInRoot() },
            ) {
                Image(
                    backdrop.image,
                    contentDescription = null,
                    modifier = Modifier.fillMaxWidth().height(WALLPAPER_HEIGHT),
                    contentScale = ContentScale.Crop,
                    alignment = Alignment.TopCenter,
                )
                Box(
                    Modifier
                        .fillMaxWidth()
                        .height(150.dp)
                        .background(
                            Brush.verticalGradient(
                                listOf(if (c.dark) Color.Black.copy(alpha = 0.55f) else c.paper.copy(alpha = 0.75f), Color.Transparent),
                            ),
                        ),
                )
            }
        }
        CompositionLocalProvider(LocalBackdrop provides backdrop) { content() }
    }
}

/**
 * A grouped pane (`.grp`): on paper, the raised plate (with a hairline ring in light); over a wallpaper, frosted glass
 * (`.grp.pf2`): the art blurred behind it, a 40 % fill, a lit top edge and a faint ring. Solid when the system reduces
 * transparency or asks for more contrast ([SignalPrefs.frosted]).
 *
 * A pane can be drawn in pieces (the items of a lazy list): [top] and [bottom] say whether this piece holds the pane's
 * top or bottom edge; the corners, the ring and the lit edge are drawn only where the pane has them, so the pieces
 * read as one pane. [radius] is the pane's corner (20).
 */
@Composable
fun Modifier.pane(radius: Dp = 20.dp, top: Boolean = true, bottom: Boolean = true, solid: Color? = null): Modifier {
    val c = LocalColors.current
    val backdrop = LocalBackdrop.current
    val frosted = LocalSignalPrefs.current.frosted && backdrop?.frosted != null
    val plate = solid ?: c.paperRaised
    val shape = androidx.compose.foundation.shape.RoundedCornerShape(
        topStart = if (top) radius else 0.dp,
        topEnd = if (top) radius else 0.dp,
        bottomStart = if (bottom) radius else 0.dp,
        bottomEnd = if (bottom) radius else 0.dp,
    )
    var at by remember { mutableStateOf(Offset.Zero) }
    val fill = if (c.dark) Color(0xFF1E1E24).copy(alpha = 0.40f) else Color.White.copy(alpha = 0.40f)
    val edge = when {
        frosted -> if (c.dark) Color.White.copy(alpha = 0.08f) else Color.White.copy(alpha = 0.40f)
        c.dark -> Color.Transparent
        else -> c.rule
    }
    val lit = if (c.dark) Color.White.copy(alpha = 0.16f) else Color.White.copy(alpha = 0.80f)
    return this
        .then(if (frosted) Modifier.onGloballyPositioned { at = it.boundsInRoot().topLeft } else Modifier)
        .drawBehind {
            val outline = shape.createOutline(size, layoutDirection, this)
            if (!frosted) {
                drawOutline(outline, plate)
            } else {
                val image = backdrop!!.frosted!!
                val path = when (outline) {
                    is androidx.compose.ui.graphics.Outline.Rectangle -> androidx.compose.ui.graphics.Path().apply { addRect(outline.rect) }
                    is androidx.compose.ui.graphics.Outline.Rounded -> androidx.compose.ui.graphics.Path().apply { addRoundRect(outline.roundRect) }
                    is androidx.compose.ui.graphics.Outline.Generic -> outline.path
                }
                clipPath(path) {
                    drawRect(c.paper)
                    val b = backdrop.bounds
                    if (b.width > 0f) {
                        translate(b.left - at.x, b.top - at.y) {
                            // The frosted copy is the art scaled down: drawn back over the art's own bounds, cropped
                            // as the art is (its full width, from the top).
                            val scale = maxOf(b.width / image.width, b.height / image.height)
                            val srcW = (b.width / scale).toInt().coerceIn(1, image.width)
                            val srcH = (b.height / scale).toInt().coerceIn(1, image.height)
                            drawImage(
                                image,
                                srcOffset = IntOffset((image.width - srcW) / 2, 0),
                                srcSize = IntSize(srcW, srcH),
                                dstOffset = IntOffset.Zero,
                                dstSize = IntSize(b.width.toInt(), b.height.toInt()),
                                filterQuality = androidx.compose.ui.graphics.FilterQuality.High,
                            )
                        }
                    }
                    drawRect(fill)
                    if (top) drawLine(lit, Offset(radius.toPx() * 0.6f, 0.5.dp.toPx()), Offset(size.width - radius.toPx() * 0.6f, 0.5.dp.toPx()), strokeWidth = 1.dp.toPx())
                }
            }
            if (edge.alpha > 0f) {
                // The ring of the whole pane: a piece draws the ring of a pane that runs past its open ends, clipped
                // to itself, so no line falls between two pieces.
                val stroke = 1.dp.toPx()
                val extend = radius.toPx() + stroke * 2
                val y0 = if (top) stroke / 2 else -extend
                val y1 = if (bottom) size.height - stroke / 2 else size.height + extend
                val ring = androidx.compose.ui.geometry.RoundRect(
                    stroke / 2, y0, size.width - stroke / 2, y1,
                    androidx.compose.ui.geometry.CornerRadius(radius.toPx() - stroke / 2),
                )
                clipRect {
                    drawPath(androidx.compose.ui.graphics.Path().apply { addRoundRect(ring) }, edge, style = androidx.compose.ui.graphics.drawscope.Stroke(stroke))
                }
            }
        }
        .clip(shape)
}

/** [pane] for a whole shape (sheets of glass that are not lists). */
@Composable
fun Modifier.pane(shape: Shape, solid: Color? = null): Modifier {
    val c = LocalColors.current
    val backdrop = LocalBackdrop.current
    if (backdrop == null || !LocalSignalPrefs.current.frosted) {
        return this.clip(shape).background(solid ?: c.paperRaised).then(if (!c.dark) Modifier.border(1.dp, c.rule, shape) else Modifier)
    }
    return pane(20.dp, solid = solid)
}
