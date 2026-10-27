package dev.reins.android.design

import androidx.compose.foundation.Canvas
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.runtime.Composable
import androidx.compose.runtime.remember
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.geometry.Size
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp
import com.google.zxing.BarcodeFormat
import com.google.zxing.EncodeHintType
import com.google.zxing.qrcode.QRCodeWriter
import com.google.zxing.qrcode.decoder.ErrorCorrectionLevel

/** The modules of [text] as a QR code (true: dark), row by row; null when it can't be encoded. */
fun qrModules(text: String): List<BooleanArray>? = try {
    // Size 0: the writer picks the smallest version; no quiet zone (the plate around it is one).
    val hints = mapOf(EncodeHintType.ERROR_CORRECTION to ErrorCorrectionLevel.M, EncodeHintType.MARGIN to 0)
    val matrix = QRCodeWriter().encode(text, BarcodeFormat.QR_CODE, 0, 0, hints)
    List(matrix.height) { y -> BooleanArray(matrix.width) { x -> matrix[x, y] } }
} catch (_: Exception) {
    null
}

/**
 * [text] as a QR code: black modules on a white plate (whatever the theme, so any camera reads it), [size] square.
 * Screen readers hear [label], never the text.
 */
@Composable
fun QrCode(text: String, modifier: Modifier = Modifier, size: Dp = 200.dp, label: String = "QR code") {
    val modules = remember(text) { qrModules(text) } ?: return
    Box(
        modifier
            .clip(RoundedCornerShape(16.dp))
            .background(Color.White)
            .padding(14.dp)
            .semantics { contentDescription = label },
    ) {
        Canvas(Modifier.size(size)) {
            val n = modules.size
            val cell = this.size.width / n
            modules.forEachIndexed { y, row ->
                row.forEachIndexed { x, dark ->
                    // A hair wider than the cell, so neighbours never leave a seam.
                    if (dark) drawRect(Color.Black, Offset(x * cell, y * cell), Size(cell + 0.5f, cell + 0.5f))
                }
            }
        }
    }
}
