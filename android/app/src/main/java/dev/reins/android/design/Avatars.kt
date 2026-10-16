package dev.reins.android.design

import android.content.Context
import android.graphics.Bitmap
import android.graphics.Canvas as AndroidCanvas
import android.util.LruCache
import androidx.compose.foundation.Canvas
import androidx.compose.foundation.Image
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.runtime.Composable
import androidx.compose.runtime.remember
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.ColorFilter
import androidx.compose.ui.graphics.Path
import androidx.compose.ui.graphics.asImageBitmap
import androidx.compose.ui.graphics.drawscope.scale
import androidx.compose.ui.graphics.vector.PathParser
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp
import com.caverock.androidsvg.SVG
import dev.reins.android.design.blobatar.Blobatar
import dev.reins.android.ui.common.untrusted
import dev.reins.core.ConnectionView

/**
 * An AI provider the user can pick as a connection's icon.
 *
 * [asset] is the provider's real logo (`assets/providers/<key>.svg`, see NOTICE-providers.md); [mono] logos are
 * single-colour and take the text colour, the others keep their brand colours.
 */
class Provider(val key: String, val name: String, val mono: Boolean, val words: List<String>) {
    val asset get() = "providers/$key.svg"
}

/**
 * Icons for connections: the logo of a well-known AI provider, or, by default, a blobatar
 * (https://github.com/Alain00/blobatar) generated from the name.
 */
object Providers {
    val all = listOf(
        Provider("claude", "Claude", mono = false, words = listOf("claude", "anthropic")),
        Provider("openai", "ChatGPT", mono = true, words = listOf("chatgpt", "gpt", "openai", "codex")),
        Provider("gemini", "Gemini", mono = false, words = listOf("gemini", "bard")),
        Provider("grok", "Grok", mono = true, words = listOf("grok", "xai")),
        Provider("hermes", "Hermes", mono = true, words = listOf("hermes")),
        Provider("perplexity", "Perplexity", mono = false, words = listOf("perplexity")),
        Provider("mistral", "Mistral", mono = false, words = listOf("mistral", "le chat")),
        Provider("deepseek", "DeepSeek", mono = false, words = listOf("deepseek")),
        Provider("copilot", "Copilot", mono = false, words = listOf("copilot")),
        Provider("cursor", "Cursor", mono = true, words = listOf("cursor")),
        Provider("qwen", "Qwen", mono = false, words = listOf("qwen")),
        Provider("kimi", "Kimi", mono = false, words = listOf("kimi", "moonshot")),
        Provider("meta", "Meta AI", mono = false, words = listOf("llama", "meta ai")),
        Provider("ollama", "Ollama", mono = true, words = listOf("ollama")),
    )

    fun byKey(key: String?): Provider? = all.firstOrNull { it.key == key }

    /** The provider a connection name suggests, if any ("My Claude" → Claude). */
    fun infer(label: String): Provider? {
        val lower = label.lowercase()
        return all.firstOrNull { p -> p.words.any { it in lower } }
    }
}

/** A computer's platform, shown by its logo (`assets/platforms/<key>.svg`, see providers/NOTICE.md). */
class Platform(val key: String, val name: String) {
    val asset get() = "platforms/$key.svg"
}

/** The platforms a computer can show: the keys the core uses ("macos", "windows", "linux"). */
object Platforms {
    val all = listOf(Platform("macos", "macOS"), Platform("windows", "Windows"), Platform("linux", "Linux"))

    fun byKey(key: String?): Platform? = all.firstOrNull { it.key == key }

    /** The Windows mark keeps its blue; the Apple logo and Tux take the text colour. */
    fun color(platform: Platform, colors: RColors): Color = if (platform.key == "windows") Color(0xFF0078D4) else colors.text
}

/** The icon key that means "draw the blobatar". */
const val BLOB_ICON = "blob"

/**
 * What Auto shows for a connection: the core's `autoIcon` (a platform's logo, a provider's or the blobatar), or, for
 * older data without one, the provider its name suggests, else its blobatar.
 */
fun autoIcon(connection: ConnectionView): String =
    connection.autoIcon.ifEmpty { Providers.infer(connection.label)?.key ?: BLOB_ICON }

/** What a connection shows: the icon the user picked, else what Auto resolves to ([autoIcon]). */
fun shownIcon(connection: ConnectionView): String = connection.icon ?: autoIcon(connection)

/** The name of an icon choice: Auto (none picked), Blobatar, a platform's or a provider's. */
fun iconName(pick: String?): String = when (pick) {
    null -> "Auto"
    BLOB_ICON -> "Blobatar"
    else -> Platforms.byKey(pick)?.name ?: Providers.byKey(pick)?.name ?: "Auto"
}

/** Rasterises bundled SVG assets once per size (AndroidSVG); single-colour logos come out black and are tinted at draw time. */
object SvgAssets {
    private val cache = LruCache<String, Bitmap>(64)
    private val parsed = HashMap<String, SVG?>()

    @Synchronized
    fun bitmap(context: Context, asset: String, px: Int): Bitmap? {
        val key = "$asset@$px"
        cache.get(key)?.let { return it }
        val svg = parsed.getOrPut(asset) {
            runCatching { context.assets.open(asset).use { SVG.getFromInputStream(it) } }.getOrNull()
        } ?: return null
        val size = px.coerceAtLeast(1)
        val bitmap = Bitmap.createBitmap(size, size, Bitmap.Config.ARGB_8888)
        svg.setDocumentWidth(size.toFloat())
        svg.setDocumentHeight(size.toFloat())
        svg.renderToCanvas(AndroidCanvas(bitmap))
        cache.put(key, bitmap)
        return bitmap
    }
}

/** [pick] is the stored choice (a provider key, "blob"), or null to derive the icon from [label]. */
@Composable
fun ConnectionAvatar(label: String, pick: String?, modifier: Modifier = Modifier, size: Dp = 40.dp) {
    val provider = Providers.byKey(pick) ?: if (pick == null) Providers.infer(label) else null
    if (provider != null) ProviderAvatar(provider, modifier, size) else BlobAvatar(label, modifier, size)
}

/**
 * A connection's icon, the same on every screen: [shown] (by default what it shows, [shownIcon]) as a provider's
 * logo, a platform's logo, or its blobatar with the seed, shape and colour it keeps (seeded by its name when it keeps
 * none).
 */
@Composable
fun ConnectionAvatar(connection: ConnectionView, modifier: Modifier = Modifier, size: Dp = 40.dp, shown: String = shownIcon(connection)) {
    val provider = Providers.byKey(shown)
    val platform = Platforms.byKey(shown)
    when {
        provider != null -> ProviderAvatar(provider, modifier, size)
        platform != null -> PlatformAvatar(platform, modifier, size)
        else -> {
            val kept = connection.blobatar
            if (kept != null) {
                BlobAvatar(kept.seed, modifier, size, shape = kept.shape.toInt(), colour = kept.colour.toInt())
            } else {
                BlobAvatar(untrusted(connection.label), modifier, size)
            }
        }
    }
}

/** The round plate every connection icon sits on, so provider logos and blobatars look alike. */
@Composable
private fun AvatarPlate(size: Dp, modifier: Modifier = Modifier, content: @Composable () -> Unit) {
    val c = LocalColors.current
    Box(
        modifier.size(size).clip(CircleShape).background(if (c.dark) Color(0xFF1D1D21) else Color.White).border(1.dp, c.hairline, CircleShape),
        contentAlignment = Alignment.Center,
    ) { content() }
}

@Composable
fun ProviderAvatar(provider: Provider, modifier: Modifier = Modifier, size: Dp = 40.dp) {
    val c = LocalColors.current
    val context = LocalContext.current
    val markSize = size * 0.56f
    val px = with(LocalDensity.current) { markSize.roundToPx() }
    val mark = remember(provider.key, px) { SvgAssets.bitmap(context, provider.asset, px)?.asImageBitmap() }
    AvatarPlate(size, modifier) {
        if (mark != null) {
            Image(
                bitmap = mark,
                contentDescription = provider.name,
                modifier = Modifier.size(markSize),
                colorFilter = if (provider.mono) ColorFilter.tint(c.text) else null,
            )
        }
    }
}

/** A computer's platform: its logo on the same round plate as the providers'. */
@Composable
fun PlatformAvatar(platform: Platform, modifier: Modifier = Modifier, size: Dp = 40.dp) {
    val c = LocalColors.current
    val context = LocalContext.current
    val markSize = size * 0.5f
    val px = with(LocalDensity.current) { markSize.roundToPx() }
    val mark = remember(platform.key, px) { SvgAssets.bitmap(context, platform.asset, px)?.asImageBitmap() }
    AvatarPlate(size, modifier) {
        if (mark != null) {
            Image(mark, platform.name, Modifier.size(markSize), colorFilter = ColorFilter.tint(Platforms.color(platform, c)))
        }
    }
}

/**
 * A blobatar: the deterministic figure the library draws for [seedText], on the same round plate as the logos.
 * [shape] (an index of [Blobatar.SHAPES]) and [colour] (an index of [Blobatar.COLOURS]) replace the seed's own.
 */
@Composable
fun BlobAvatar(seedText: String, modifier: Modifier = Modifier, size: Dp = 40.dp, shape: Int? = null, colour: Int? = null) {
    val marks = remember(seedText, shape, colour) { Blobs.marks(seedText.ifBlank { "?" }, shape, colour) }
    AvatarPlate(size, modifier) {
        Canvas(Modifier.size(size * 0.92f)) {
            scale(this.size.minDimension / 100f, pivot = Offset.Zero) {
                for (mark in marks) {
                    if (mark.path != null) drawPath(mark.path, mark.color) else drawCircle(mark.color, mark.radius, mark.center)
                }
            }
        }
    }
}

/** One mark of a blobatar, ready to draw in its 100 × 100 box: a path, or a circle when [path] is null. */
private class BlobMark(val path: Path?, val center: Offset, val radius: Float, val color: Color)

/**
 * Blobatars by seed, shape and colour, generated, parsed and coloured once and shared by every avatar that shows them:
 * a list that scrolls a row back into view draws it straight away. The paths are never changed after they are built.
 */
private object Blobs {
    private val cache = LruCache<String, List<BlobMark>>(160)

    fun marks(seed: String, shape: Int? = null, colour: Int? = null): List<BlobMark> {
        val key = "${shape ?: ""}/${colour ?: ""}/$seed"
        return cache.get(key) ?: build(seed, shape, colour).also { cache.put(key, it) }
    }

    private fun build(seed: String, shape: Int?, colour: Int?): List<BlobMark> {
        val parser = PathParser()
        val head = colour?.let { Blobatar.COLOURS.getOrNull(it) }
        val band = shape?.takeIf { it in Blobatar.SHAPES.indices }
        return Blobatar.figure(seed, Blobatar.Backdrop.NONE, band, head).marks.map { mark ->
            val color = Color(android.graphics.Color.parseColor(mark.fill))
            when (mark) {
                is Blobatar.Mark.Path -> BlobMark(parser.parsePathString(mark.d).toPath(), Offset.Zero, 0f, color)
                is Blobatar.Mark.Circle -> BlobMark(null, Offset(mark.cx.toFloat(), mark.cy.toFloat()), mark.r.toFloat(), color)
            }
        }
    }
}

/** A connected service (Gmail) as its logo on a plate. */
@Composable
fun ServiceAvatar(service: String, modifier: Modifier = Modifier, size: Dp = 40.dp) {
    val c = LocalColors.current
    val context = LocalContext.current
    val markSize = size * 0.5f
    val px = with(LocalDensity.current) { markSize.roundToPx() }
    val mark = remember(service, px) { SvgAssets.bitmap(context, "services/$service.svg", px)?.asImageBitmap() }
    AvatarPlate(size, modifier) {
        if (mark != null) {
            Image(mark, service, Modifier.size(markSize), colorFilter = ColorFilter.tint(serviceColor(service, c)))
        } else {
            GlyphIcon(Glyph.Link, c.secondary, size = markSize)
        }
    }
}

/** The brand colour of a service's mark. */
fun serviceColor(service: String, colors: RColors): Color = when (service) {
    "gmail" -> Color(0xFFEA4335)
    "telegram" -> Color(0xFF26A5E4)
    "github" -> colors.text
    "gitlab" -> Color(0xFFFC6D26)
    "codeberg" -> Color(0xFF2185D0)
    "bitbucket" -> Color(0xFF0052CC)
    "mcp" -> colors.accent
    "gcalendar" -> Color(0xFF4285F4)
    "gcontacts" -> Color(0xFF4285F4)
    "device_calendar" -> Color(0xFF34A853)
    "device_contacts" -> Color(0xFFF57C00)
    "sms" -> Color(0xFF00A884)
    "vault" -> Color(0xFF175DDC)
    "payments" -> Color(0xFF0E9F6E)
    else -> colors.secondary
}
