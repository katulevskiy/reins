package dev.reins.android.ui.autopilot

import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.width
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.unit.dp
import dev.reins.android.design.Glyph
import dev.reins.android.design.GlyphIcon
import dev.reins.android.design.LocalColors
import dev.reins.android.design.RText
import dev.reins.android.design.RType
import dev.reins.android.design.SignalDialog

/**
 * What a change to Auto would have done on this phone's own last 30 days (Decisions v2, 02 §4): "Last 30 days: 12
 * Auto → Ask, 0 Ask → Auto". Worked out on the phone; nothing is sent.
 */
@Composable
fun PreviewLine(line: String, modifier: Modifier = Modifier) {
    val c = LocalColors.current
    Row(
        modifier.fillMaxWidth().padding(horizontal = 16.dp, vertical = 10.dp).testTag("previewLine"),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        GlyphIcon(Glyph.Activity, c.inkMeta, size = 16.dp)
        Spacer(Modifier.width(10.dp))
        RText(line, RType.sans(14f), c.secondary, Modifier.weight(1f).testTag("previewLineText"), maxLines = 2)
    }
}

/** Before a matrix cell gets looser: its preview, then Continue (the owner's check follows) or Cancel. */
@Composable
fun LoosenDialog(loosen: LoosenCell, onConfirm: () -> Unit, onDismiss: () -> Unit) {
    SignalDialog(
        title = "Let Auto do more",
        text = loosen.preview.ifBlank { "No history yet" },
        confirmLabel = "Continue",
        onConfirm = onConfirm,
        onDismiss = onDismiss,
        confirmTag = "loosenConfirm",
        glyph = Glyph.Autopilot,
    )
}
