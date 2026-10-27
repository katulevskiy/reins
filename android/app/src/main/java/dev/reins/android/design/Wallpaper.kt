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
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.runtime.Composable
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.runtime.LaunchedEffect
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
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext

/**
 * The images behind Activity (Reins Next, "Activity with a wallpaper"): dithered art that fades into the paper at its
 * foot. Halo is the dark default, Torii dither the light one; any of them (or the owner's own photo) can be picked for
 * either theme, and any of them takes any effect and fade ([WallpaperChoice]).
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

    /** The owner's own photo, one per theme: fitted and kept as it is, so its effect and fade can change later. */
    Photo("Your photo", null),
    ;

    /** The image as it is (no effect), decoded once per process (the owner's photo is read again after it changes). */
    fun bitmap(context: Context, dark: Boolean): Bitmap? = Wallpapers.base(context, this, dark)

    internal fun assetPath(): String? = asset?.let { "wallpapers/$it" }

    companion object {
        fun named(name: String?): Wallpaper? = entries.firstOrNull { it.name == name }

        /** The shipped images, in the picker's order. */
        val shipped: List<Wallpaper> get() = entries.filter { it != Photo }
    }
}

/**
 * The pictures behind Activity, made on this phone and cached per process. A shipped image as it is is the asset
 * itself; any other choice is [PhotoEffects.apply] on the image (or the owner's photo), made off the main thread and
 * kept in a file keyed by image, effect, fade and theme ([key]). The frosted copies are made from the same picture.
 */
object Wallpapers {
    private const val KEPT = 4
    private val bases = HashMap<String, Bitmap>()
    private val made = object : LinkedHashMap<String, Bitmap>(8, 0.75f, true) {
        override fun removeEldestEntry(eldest: MutableMap.MutableEntry<String, Bitmap>?) = size > KEPT
    }
    private val frosted = object : LinkedHashMap<String, Bitmap>(16, 0.75f, true) {
        override fun removeEldestEntry(eldest: MutableMap.MutableEntry<String, Bitmap>?) = size > 16
    }

    /** Bumped when the owner's photo changes, so screens read it again. */
    var photoVersion by mutableStateOf(0)

    /** The owner's photo for a theme as picked: fitted to 780 × 1000, no effect (JPEG). */
    fun sourceFile(context: Context, dark: Boolean): File = File(context.filesDir, if (dark) "wallpaper-source-dark.jpg" else "wallpaper-source-light.jpg")

    /** Before effects were separate: the owner's photo with its effect and fade baked in. Shown as it is. */
    fun bakedFile(context: Context, dark: Boolean): File = File(context.filesDir, if (dark) "wallpaper-dark.jpg" else "wallpaper-light.jpg")

    fun hasPhoto(context: Context, dark: Boolean): Boolean = sourceFile(context, dark).isFile || bakedFile(context, dark).isFile

    /** Where made pictures are kept. */
    fun madeDir(context: Context): File = File(context.cacheDir, "wallpapers")

    /** The name a made picture is kept under: one per image, effect, fade step and theme. */
    fun key(choice: WallpaperChoice, dark: Boolean): String =
        "${choice.image.name}-${choice.effect.name}-${choice.fadeStep}-${if (dark) "dark" else "light"}"

    fun madeFile(context: Context, choice: WallpaperChoice, dark: Boolean): File = File(madeDir(context), key(choice, dark) + ".png")

    /** Keeps the owner's new photo for a theme ([PhotoEffects.fit] already), in place of the old one. */
    fun savePhoto(context: Context, dark: Boolean, fitted: Bitmap) {
        sourceFile(context, dark).outputStream().use { fitted.compress(Bitmap.CompressFormat.JPEG, 92, it) }
        bakedFile(context, dark).delete()
        photoChanged(context)
    }

    fun photoChanged(context: Context) {
        val photo = Wallpaper.Photo.name + "-"
        synchronized(this) {
            bases.keys.filter { it.startsWith("photo:") }.forEach { bases.remove(it) }
            made.keys.filter { it.startsWith(photo) }.forEach { made.remove(it) }
            frosted.keys.filter { it.startsWith(photo) }.forEach { frosted.remove(it) }
        }
        madeDir(context).listFiles()?.filter { it.name.startsWith(photo) }?.forEach { it.delete() }
        photoVersion++
    }

    /** Drops every picture held in memory (tests). */
    internal fun forget() = synchronized(this) {
        bases.clear()
        made.clear()
        frosted.clear()
    }

    /** The image as it is: the asset, or the owner's photo (its source, else the old baked file). */
    fun base(context: Context, image: Wallpaper, dark: Boolean): Bitmap? = synchronized(this) {
        val k = if (image == Wallpaper.Photo) "photo:$dark" else image.name
        bases[k] ?: runCatching {
            val path = image.assetPath()
            if (path != null) {
                context.assets.open(path).use { BitmapFactory.decodeStream(it) }
            } else {
                listOf(sourceFile(context, dark), bakedFile(context, dark)).firstOrNull { it.isFile }?.let { BitmapFactory.decodeFile(it.absolutePath) }
            }
        }.getOrNull()?.also { bases[k] = it }
    }

    /** The picture if it can be had at once: the image as it is, or one already made. Null: [bitmap] makes it. */
    fun ready(context: Context, choice: WallpaperChoice, dark: Boolean): Bitmap? =
        if (choice.plain) base(context, choice.image, dark) else synchronized(this) { made[key(choice, dark)] }

    /** The picture for [choice]: made (or read back from its file) when it is not the image as it is. Off the main thread. */
    fun bitmap(context: Context, choice: WallpaperChoice, dark: Boolean): Bitmap? {
        ready(context, choice, dark)?.let { return it }
        if (choice.plain) return null
        val k = key(choice, dark)
        val file = madeFile(context, choice, dark)
        val version = photoVersion
        val out = file.takeIf { it.isFile }?.let { f ->
            runCatching { BitmapFactory.decodeFile(f.absolutePath) }.getOrNull()?.also { f.setLastModified(System.currentTimeMillis()) }
        } ?: run {
            val src = base(context, choice.image, dark) ?: return null
            PhotoEffects.apply(src, choice.effect, choice.fadeStep / 100f, dark).also {
                // A photo replaced while this was made: shown this once, not kept as the new photo's.
                if (choice.image != Wallpaper.Photo || version == photoVersion) keep(context, file, it, dark)
            }
        }
        if (choice.image != Wallpaper.Photo || version == photoVersion) synchronized(this) { made[k] = out }
        return out
    }

    /** Writes a made picture and drops the theme's oldest beyond the last few. */
    private fun keep(context: Context, file: File, bmp: Bitmap, dark: Boolean) {
        runCatching {
            file.parentFile?.mkdirs()
            val tmp = File(file.parentFile, file.name + ".tmp")
            tmp.outputStream().use { bmp.compress(Bitmap.CompressFormat.PNG, 100, it) }
            tmp.renameTo(file)
            val theme = if (dark) "-dark.png" else "-light.png"
            madeDir(context).listFiles()?.filter { it.name.endsWith(theme) }?.sortedByDescending { it.lastModified() }?.drop(KEPT)?.forEach { it.delete() }
        }
    }

    /**
     * The picture as a pane sees it through the glass: a small copy, box-blurred, saturated 1.9× and brightened 1.06
     * (light) or darkened to 0.82 (dark): the boards' `backdrop-filter: blur(40px) saturate(1.9) brightness(…)`.
     * Drawn back at full size it is soft; the fine detail that fights text is gone. [src] is the picture for [choice].
     */
    fun frosted(choice: WallpaperChoice, dark: Boolean, src: Bitmap): Bitmap {
        val k = key(choice, dark)
        synchronized(this) { frosted[k]?.let { return it } }
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

/** A wallpaper's picture and the choice it was made for. */
class WallpaperArt(val choice: WallpaperChoice, val dark: Boolean, val bitmap: Bitmap)

/**
 * The picture for [choice] in a theme: at once when it is the image as it is (or was made before), else made off the
 * main thread; the last picture stays until the new one is ready.
 */
@Composable
fun rememberWallpaper(choice: WallpaperChoice?, dark: Boolean): WallpaperArt? {
    val context = LocalContext.current
    val version = Wallpapers.photoVersion
    val now = remember(choice, dark, version) { choice?.let { ch -> Wallpapers.ready(context, ch, dark)?.let { WallpaperArt(ch, dark, it) } } }
    var last by remember { mutableStateOf(now) }
    LaunchedEffect(choice, dark, version) {
        last = when {
            choice == null -> null
            now != null -> now
            else -> withContext(Dispatchers.Default) { Wallpapers.bitmap(context, choice, dark) }?.let { WallpaperArt(choice, dark, it) }
        }
    }
    return if (choice == null) null else now ?: last
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
fun WallpaperPage(wallpaper: WallpaperChoice?, modifier: Modifier = Modifier, artHeight: Dp? = WALLPAPER_HEIGHT, content: @Composable () -> Unit) {
    val c = LocalColors.current
    val art = rememberWallpaper(wallpaper, c.dark)
    val backdrop = remember(art) { art?.let { Backdrop(it.bitmap.asImageBitmap(), Wallpapers.frosted(it.choice, it.dark, it.bitmap).asImageBitmap()) } }
    Box(modifier.background(c.paper)) {
        if (backdrop != null) {
            // [artHeight] null: the art fills the page (the welcome).
            val art = if (artHeight == null) Modifier.fillMaxSize() else Modifier.fillMaxWidth().height(artHeight)
            Box(
                art.onGloballyPositioned { backdrop.bounds = it.boundsInRoot() },
            ) {
                Image(
                    backdrop.image,
                    contentDescription = null,
                    modifier = art,
                    contentScale = ContentScale.Crop,
                    alignment = Alignment.TopCenter,
                )
                if (artHeight != null) Box(
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
