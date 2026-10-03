package dev.rewarden.android.ui.common

import android.graphics.BitmapFactory
import androidx.compose.foundation.Image
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.runtime.Composable
import androidx.compose.runtime.remember
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.asImageBitmap
import androidx.compose.ui.layout.ContentScale
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import dev.rewarden.android.design.Card
import dev.rewarden.android.design.Glyph
import dev.rewarden.android.design.GlyphIcon
import dev.rewarden.android.design.LocalColors
import dev.rewarden.android.design.RText
import dev.rewarden.android.design.RType
import dev.rewarden.core.BlobView

/** How many lines of a text file are shown. */
private const val TEXT_LINES = 12

/**
 * A file the server holds, as it saw the bytes: its name (from the AI), size, type and SHA-256, and a preview: the start
 * of a text file, an image, or what kind of file it is.
 */
@Composable
fun FileCard(blob: BlobView, modifier: Modifier = Modifier) {
    val c = LocalColors.current
    Card(modifier) {
        Column(Modifier.padding(16.dp), verticalArrangement = Arrangement.spacedBy(6.dp)) {
            Row(verticalAlignment = Alignment.CenterVertically) {
                GlyphIcon(Glyph.Tray, c.secondary, size = 20.dp)
                Spacer(Modifier.width(10.dp))
                RText(
                    untrusted(blob.name).ifEmpty { "(no name)" },
                    RType.sans(16.5f, FontWeight.SemiBold),
                    c.text,
                    Modifier.weight(1f).testTag("fileName"),
                    maxLines = 2,
                    overflow = TextOverflow.MiddleEllipsis,
                )
            }
            RText(
                fileSize(blob.size) + " · " + untrusted(blob.contentType),
                RType.sans(13.5f),
                c.secondary,
                Modifier.testTag("fileSize"),
                maxLines = 1,
            )
            RText("SHA-256 " + shortSha(blob.sha256), RType.mono(12.5f), c.tertiary, Modifier.testTag("fileSha"), maxLines = 1, ltr = true)
            Preview(blob)
        }
    }
}

@Composable
private fun Preview(blob: BlobView) {
    val c = LocalColors.current
    val image = remember(blob.id, blob.previewImage) {
        blob.previewImage?.let { bytes -> runCatching { BitmapFactory.decodeByteArray(bytes, 0, bytes.size)?.asImageBitmap() }.getOrNull() }
    }
    val text = blob.previewText?.let(::untrusted)?.takeIf { it.isNotBlank() }
    when {
        image != null -> Image(
            image,
            contentDescription = "Preview of ${untrusted(blob.name)}",
            modifier = Modifier
                .padding(top = 6.dp)
                .fillMaxWidth()
                .heightIn(max = 240.dp)
                .clip(RoundedCornerShape(12.dp))
                .background(c.controlFill)
                .testTag("fileImage"),
            contentScale = ContentScale.Fit,
        )
        text != null && isTextFile(blob.contentType) -> RText(
            text,
            RType.mono(12.5f),
            c.text,
            Modifier
                .padding(top = 6.dp)
                .fillMaxWidth()
                .background(c.controlFill, RoundedCornerShape(12.dp))
                .padding(12.dp)
                .testTag("fileText"),
            maxLines = TEXT_LINES,
            ltr = true,
        )
        text != null -> RText(text, RType.sans(14f, FontWeight.Medium), c.secondary, Modifier.padding(top = 4.dp).testTag("fileKind"), maxLines = 2)
        else -> Unit
    }
}
