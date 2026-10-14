package dev.reins.android.ui.common

import androidx.compose.foundation.Image
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.offset
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.rounded.Article
import androidx.compose.material.icons.automirrored.rounded.Send
import androidx.compose.material.icons.rounded.Computer
import androidx.compose.material.icons.rounded.Delete
import androidx.compose.material.icons.rounded.Edit
import androidx.compose.material.icons.rounded.Key
import androidx.compose.material.icons.rounded.PlayArrow
import androidx.compose.material.icons.rounded.QuestionMark
import androidx.compose.material.icons.rounded.Search
import androidx.compose.material.icons.rounded.Terminal
import androidx.compose.material.icons.rounded.Upload
import androidx.compose.material.icons.rounded.VerifiedUser
import androidx.compose.material.icons.rounded.Visibility
import androidx.compose.material3.Icon
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.remember
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.ColorFilter
import androidx.compose.ui.graphics.asImageBitmap
import androidx.compose.ui.graphics.lerp
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.text.SpanStyle
import androidx.compose.ui.text.buildAnnotatedString
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.text.withStyle
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp
import dev.reins.android.design.Fonts
import dev.reins.android.design.LocalColors
import dev.reins.android.design.RColors
import dev.reins.android.design.RType
import dev.reins.android.design.SvgAssets
import dev.reins.android.design.serviceName
import dev.reins.android.design.serviceTint
import dev.reins.android.design.tileEdge
import dev.reins.android.design.tilePlate
import dev.reins.core.RequestLabel

// What a request is, drawn the same in every Activity row, waiting card and entry: the thing it touches as the tile
// (a terminal, a file, a key, a brand's logo) with what it does as a badge on its corner, and the request itself as the
// title ("Run `git push --force origin main`"). The core decides all of it (`RequestLabel`); this only draws.

/** The title as one line of plain text. */
fun RequestLabel.plain(): String = buildString {
    append(lead)
    if (subject.isNotEmpty()) {
        if (isNotEmpty()) append(' ')
        append(subject)
    }
    append(tail)
}

/** The provider's own icon when it has no logo: a terminal, a file, a key, a computer. */
fun providerIcon(provider: String): ImageVector? = when (provider) {
    "terminal", "ssh" -> Icons.Rounded.Terminal
    "file" -> Icons.AutoMirrored.Rounded.Article
    "secrets" -> Icons.Rounded.Key
    "desktop" -> Icons.Rounded.Computer
    else -> null
}

/** The logo's asset: "mcp" for any MCP server, else the integration's own. */
fun providerAsset(provider: String): String? = when {
    providerIcon(provider) != null -> null
    provider.startsWith("mcp:") -> "mcp"
    provider.isBlank() -> null
    else -> provider
}

fun providerTint(provider: String, c: RColors): Color = when (provider) {
    "secrets" -> c.send
    "ssh" -> c.read
    "terminal", "file", "desktop" -> c.text
    else -> serviceTint(providerAsset(provider) ?: provider, c)
}

/** What the badge on the tile's corner shows. */
fun actionIcon(action: String): ImageVector = when (action) {
    "run" -> Icons.Rounded.PlayArrow
    "read" -> Icons.Rounded.Visibility
    "send" -> Icons.AutoMirrored.Rounded.Send
    "delete" -> Icons.Rounded.Delete
    "secrets" -> Icons.Rounded.Key
    "push" -> Icons.Rounded.Upload
    "search" -> Icons.Rounded.Search
    "grant" -> Icons.Rounded.VerifiedUser
    "ask" -> Icons.Rounded.QuestionMark
    else -> Icons.Rounded.Edit
}

fun actionTint(action: String, c: RColors): Color = when (action) {
    "read" -> c.read
    "delete" -> c.danger
    "secrets" -> c.warning
    "push", "grant" -> if (c.dark) Color(0xFFA99BFA) else c.accent
    "search" -> c.search
    "ask" -> c.secondary
    else -> c.send
}

/** "Run", "Read", ... for a screen reader, with the provider's name. */
fun actionWords(label: RequestLabel): String = "${label.action.replaceFirstChar { it.uppercase() }}, ${providerName(label.provider)}"

fun providerName(provider: String): String = when (provider) {
    "terminal" -> "Terminal"
    "file" -> "File"
    "secrets" -> "Secrets"
    "ssh" -> "SSH"
    "desktop" -> "Computer"
    else -> serviceName(provider)
}

/**
 * The tile: the provider on a round plate, the action on a small badge at its bottom right, ringed in [ring] (the colour
 * behind the tile) so the two read as one.
 */
@Composable
fun ProviderTile(label: RequestLabel, modifier: Modifier = Modifier, size: Dp = 40.dp, ring: Color = LocalColors.current.background) {
    val c = LocalColors.current
    val context = LocalContext.current
    val mark = size * 0.5f
    val px = with(LocalDensity.current) { mark.roundToPx() }
    val asset = providerAsset(label.provider)
    val logo = remember(asset, px) { asset?.let { SvgAssets.bitmap(context, "services/$it.svg", px)?.asImageBitmap() } }
    val tint = providerTint(label.provider, c)
    Box(modifier.size(size).testTag("providerTile:${label.provider}:${label.action}")) {
        Box(
            Modifier.size(size).clip(CircleShape).background(tilePlate(c)).border(1.dp, tileEdge(c), CircleShape),
            contentAlignment = Alignment.Center,
        ) {
            when {
                logo != null -> Image(logo, providerName(label.provider), Modifier.size(mark), colorFilter = ColorFilter.tint(tint))
                else -> Icon(providerIcon(label.provider) ?: Icons.Rounded.Computer, providerName(label.provider), Modifier.size(mark), tint = tint)
            }
        }
        val badge = size * 0.5f
        val action = actionTint(label.action, c)
        Box(
            Modifier
                .align(Alignment.BottomEnd)
                .offset(x = size * 0.1f, y = size * 0.1f)
                .size(badge)
                .clip(RoundedCornerShape(badge * 0.35f))
                .background(ring)
                .border(2.dp, ring, RoundedCornerShape(badge * 0.35f))
                .testTag("actionBadge:${label.action}"),
            contentAlignment = Alignment.Center,
        ) {
            Box(
                Modifier.size(badge - 4.dp).clip(RoundedCornerShape(badge * 0.28f)).background(lerp(tilePlate(c), action, 0.22f)),
                contentAlignment = Alignment.Center,
            ) { Icon(actionIcon(label.action), null, Modifier.size(badge * 0.55f), tint = action) }
        }
    }
}

/**
 * The title: the lead in the text colour, the subject set apart (a command or a path in mono, a name in semibold), then
 * the tail. Everything from the AI is passed through [untrusted].
 */
@Composable
fun LabelText(
    label: RequestLabel,
    modifier: Modifier = Modifier,
    size: Float = 15f,
    lineHeight: Float = size * 1.3f,
    maxLines: Int = 2,
    align: TextAlign? = null,
    color: Color = LocalColors.current.text,
) {
    val c = LocalColors.current
    val text = remember(label, c, size, color) {
        buildAnnotatedString {
            val lead = untrusted(label.lead)
            append(lead)
            if (label.subject.isNotEmpty()) {
                if (lead.isNotEmpty()) append(' ')
                val style = if (label.code) {
                    SpanStyle(fontFamily = Fonts.mono, fontSize = RType.sans(size * 0.9f).fontSize, fontWeight = FontWeight.Medium)
                } else {
                    SpanStyle(fontWeight = FontWeight.SemiBold)
                }
                withStyle(style) { append(untrusted(label.subject)) }
            }
            append(untrustedLine(label.tail))
        }
    }
    Text(
        text,
        modifier,
        color = color,
        style = RType.sans(size, FontWeight.Medium, lineHeight = lineHeight),
        maxLines = maxLines,
        overflow = TextOverflow.Ellipsis,
        textAlign = align,
    )
}
