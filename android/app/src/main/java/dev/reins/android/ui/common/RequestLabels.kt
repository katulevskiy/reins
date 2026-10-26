package dev.reins.android.ui.common

import androidx.compose.ui.graphics.asAndroidPath
import android.content.Context
import android.graphics.Bitmap
import android.graphics.Canvas as AndroidCanvas
import android.graphics.Paint
import android.graphics.PorterDuff
import android.graphics.PorterDuffColorFilter
import androidx.compose.foundation.Image
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.offset
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.BasicText
import androidx.compose.runtime.Composable
import androidx.compose.runtime.remember
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.ColorFilter
import androidx.compose.ui.graphics.asImageBitmap
import androidx.compose.ui.graphics.toArgb
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.SpanStyle
import androidx.compose.ui.text.TextStyle
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp
import dev.reins.android.design.Glyph
import dev.reins.android.design.GlyphIcon
import dev.reins.android.design.LocalColors
import dev.reins.android.design.RColors
import dev.reins.android.design.RText
import dev.reins.android.design.RType
import dev.reins.android.design.SvgAssets
import dev.reins.android.design.askString
import dev.reins.android.design.serviceName
import dev.reins.android.design.serviceTint
import dev.reins.android.ui.activity.askTitle
import dev.reins.core.RequestLabel

// What a request is, drawn the same in every Activity row, waiting card, entry and notification (ASK-TEXT.md): the
// integration's mark on its white plate with the action as a round badge at its bottom-right corner, filled with the
// kind's field colour, the glyph in white, ringed in the surface behind; and the request in words ([AskSentence]).

/** The title as one line of plain text. */
fun RequestLabel.plain(): String = buildString {
    append(lead)
    if (subject.isNotEmpty()) {
        if (isNotEmpty()) append(' ')
        append(subject)
    }
    append(tail)
}

/** The logo's asset: "mcp" for any MCP server, else the integration's own; null for a local kind (a glyph). */
fun providerAsset(provider: String): String? = when {
    provider in LOCAL -> null
    provider.startsWith("mcp:") || provider == "mcp" -> "mcp"
    provider.isBlank() -> null
    else -> provider
}

private val LOCAL = setOf("terminal", "file", "files", "secrets", "ssh", "desktop")

/** The glyph of a local kind: a terminal for commands, a file, a lock for secrets, a key for SSH, a question. */
fun providerGlyph(provider: String, action: String): Glyph = when (provider) {
    "terminal" -> Glyph.Terminal
    "file", "files" -> Glyph.FileText
    "secrets" -> Glyph.Lock
    "ssh" -> Glyph.Key
    else -> when (action) {
        "ask" -> Glyph.Help
        "join" -> Glyph.Phone
        else -> Glyph.Laptop
    }
}

/** The action on the badge: a branch for a push, an eye for a read, a trash can for a delete, … */
fun actionGlyph(action: String): Glyph = when (action) {
    "run" -> Glyph.Play
    "read" -> Glyph.Eye
    "send" -> Glyph.Send
    "delete" -> Glyph.Trash
    "secrets" -> Glyph.Key
    "push" -> Glyph.GitBranch
    "search" -> Glyph.Search
    "grant" -> Glyph.ShieldCheck
    "ask" -> Glyph.Help
    "accounts" -> Glyph.People
    "pay" -> Glyph.Card
    "pair" -> Glyph.Link
    "join" -> Glyph.Phone
    "create" -> Glyph.Plus
    else -> Glyph.Pencil
}

/** "Run, Terminal" for a screen reader. */
fun actionWords(provider: String, action: String): String = "${action.replaceFirstChar { it.uppercase() }}, ${providerName(provider)}"

fun providerName(provider: String): String = when (provider) {
    "terminal" -> "Terminal"
    "file", "files" -> "File"
    "secrets" -> "Secrets"
    "ssh" -> "SSH"
    "desktop" -> "Computer"
    else -> serviceName(provider)
}

/** The mark's colour on the white plate: the brand's (as in light), ink for a local kind. */
private fun markTint(provider: String): Color {
    val light = RColors.Light
    val asset = providerAsset(provider) ?: return light.ink
    return serviceTint(asset, light)
}

/**
 * The integration's mark with the action at its corner. [ring] is the surface behind the icon: the badge is ringed in
 * it so the two read apart. [field] overrides the badge's colour (a request asked every time is red).
 */
@Composable
fun IntegrationIcon(
    provider: String,
    action: String,
    modifier: Modifier = Modifier,
    size: Dp = 40.dp,
    ring: Color = LocalColors.current.paper,
    field: dev.reins.android.design.Field = actionField(action),
) {
    val context = LocalContext.current
    val mark = size * 0.52f
    val px = with(LocalDensity.current) { mark.roundToPx() }
    val asset = providerAsset(provider)
    val logo = remember(asset, px) { asset?.let { SvgAssets.bitmap(context, "services/$it.svg", px)?.asImageBitmap() } }
    val tint = markTint(provider)
    val words = actionWords(provider, action)
    Box(modifier.size(size).semantics { contentDescription = words }.testTag("integrationIcon:$provider:$action")) {
        Box(
            // Next: a white tile with rounded corners (11 on 36), ringed faintly, the same in both themes.
            Modifier.size(size).clip(RoundedCornerShape(size * 0.3f)).background(Color.White).border(1.dp, Color.Black.copy(alpha = 0.08f), RoundedCornerShape(size * 0.3f)),
            contentAlignment = Alignment.Center,
        ) {
            if (logo != null) {
                Image(logo, null, Modifier.size(mark), colorFilter = ColorFilter.tint(tint))
            } else {
                GlyphIcon(providerGlyph(provider, action), tint, size = mark)
            }
        }
        val badge = size * 0.46f
        Box(
            Modifier
                .align(Alignment.BottomEnd)
                .offset(x = size * 0.14f + 2.dp, y = size * 0.14f + 2.dp)
                .size(badge + 4.dp)
                .clip(CircleShape)
                .background(ring)
                .padding(2.dp)
                .clip(CircleShape)
                .background(field.fill())
                .testTag("actionBadge:$action"),
            contentAlignment = Alignment.Center,
        ) { GlyphIcon(actionGlyph(action), Color.White, size = badge * 0.58f, weight = 2f) }
    }
}

/** The same icon for a notification's large icon, drawn without Compose. */
object IntegrationBitmap {
    fun draw(context: Context, provider: String, action: String, px: Int, ring: Int = android.graphics.Color.WHITE): Bitmap {
        val size = px.toFloat()
        val bitmap = Bitmap.createBitmap(px, px, Bitmap.Config.ARGB_8888)
        val canvas = AndroidCanvas(bitmap)
        val paint = Paint(Paint.ANTI_ALIAS_FLAG)
        // The plate: a little smaller than the bitmap so the badge stays inside it.
        val plate = size * 0.86f
        paint.color = android.graphics.Color.WHITE
        canvas.drawCircle(plate / 2, plate / 2, plate / 2, paint)
        paint.style = Paint.Style.STROKE
        paint.strokeWidth = size * 0.012f
        paint.color = 0x14000000
        canvas.drawCircle(plate / 2, plate / 2, plate / 2 - paint.strokeWidth / 2, paint)
        paint.style = Paint.Style.FILL
        val mark = (plate * 0.52f).toInt().coerceAtLeast(1)
        val tint = markTint(provider).toArgb()
        val asset = providerAsset(provider)
        val logo = asset?.let { SvgAssets.bitmap(context, "services/$it.svg", mark) }
        val at = (plate - mark) / 2
        if (logo != null) {
            canvas.drawBitmap(logo, at, at, Paint(Paint.ANTI_ALIAS_FLAG).apply { colorFilter = PorterDuffColorFilter(tint, PorterDuff.Mode.SRC_IN) })
        } else {
            drawGlyph(canvas, providerGlyph(provider, action), at, mark.toFloat(), tint, 1.6f)
        }
        // The badge: ringed, filled with the field, the glyph in white.
        val badge = size * 0.46f
        val cx = size - badge / 2 - size * 0.01f
        val cy = cx
        paint.color = ring
        canvas.drawCircle(cx, cy, badge / 2 + size * 0.035f, paint)
        paint.color = actionField(action).fill().toArgb()
        canvas.drawCircle(cx, cy, badge / 2, paint)
        val g = badge * 0.58f
        drawGlyph(canvas, actionGlyph(action), cx - g / 2, g, android.graphics.Color.WHITE, 2f)
        return bitmap
    }

    private fun drawGlyph(canvas: AndroidCanvas, glyph: Glyph, x: Float, size: Float, color: Int, weight: Float) {
        val k = size / 24f
        canvas.save()
        canvas.translate(x, x)
        canvas.scale(k, k)
        val paint = Paint(Paint.ANTI_ALIAS_FLAG).apply {
            this.color = color
            style = Paint.Style.FILL
        }
        // Material Symbols are filled shapes; the part's path is already on the 24 grid.
        for (part in glyph.parts) canvas.drawPath(part.path.asAndroidPath(), paint)
        canvas.restore()
    }
}

/**
 * A request in words (ASK-TEXT.md): the sentence in the serif with its one italic target, and the machine details in
 * mono on their own line under it. Rows cap the lines; the waiting field and entry pages show it whole.
 */
@Composable
fun AskSentenceText(
    sentence: AskSentence,
    modifier: Modifier = Modifier,
    style: TextStyle = RType.serif(19f, 23f),
    color: Color = LocalColors.current.ink,
    detailsColor: Color = LocalColors.current.inkMeta,
    detailsStyle: TextStyle = RType.mono(12.5f),
    maxLines: Int = Int.MAX_VALUE,
    detailsMaxLines: Int = Int.MAX_VALUE,
    tag: String? = null,
    detailsTag: String? = null,
    /** Words searched (lower case): where the sentence matches them is set apart in [highlight]. */
    terms: List<String> = emptyList(),
    highlight: SpanStyle? = null,
) {
    Column(modifier) {
        val text = remember(sentence.text, sentence.target, terms, highlight) { askTitle(sentence.text, sentence.target, terms, highlight) }
        BasicText(
            text,
            if (tag != null) Modifier.testTag(tag) else Modifier,
            style = style.copy(color = color),
            maxLines = maxLines,
            overflow = TextOverflow.Ellipsis,
        )
        sentence.details?.let {
            RText(
                it,
                detailsStyle,
                detailsColor,
                Modifier.padding(top = 3.dp).then(if (detailsTag != null) Modifier.testTag(detailsTag) else Modifier),
                maxLines = detailsMaxLines,
                ltr = true,
            )
        }
    }
}
