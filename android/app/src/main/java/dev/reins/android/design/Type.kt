package dev.reins.android.design

import androidx.compose.foundation.text.BasicText
import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.text.PlatformTextStyle
import androidx.compose.ui.text.TextStyle
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.LineHeightStyle
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.text.style.TextDirection
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.TextUnit
import androidx.compose.ui.unit.sp

/** Type: Geist for the UI, Geist Mono for addresses, codes and numbers. */
object RType {
    private val trim = LineHeightStyle(LineHeightStyle.Alignment.Center, LineHeightStyle.Trim.None)

    fun sans(size: Float, weight: FontWeight = FontWeight.Normal, lineHeight: Float? = null): TextStyle = TextStyle(
        fontFamily = Fonts.sans,
        fontWeight = weight,
        fontSize = size.sp,
        lineHeight = lineHeight?.sp ?: TextUnit.Unspecified,
        platformStyle = PlatformTextStyle(includeFontPadding = false),
        lineHeightStyle = trim,
    )

    fun mono(size: Float, weight: FontWeight = FontWeight.Normal): TextStyle = TextStyle(
        fontFamily = Fonts.mono,
        fontWeight = weight,
        fontSize = size.sp,
        platformStyle = PlatformTextStyle(includeFontPadding = false),
        lineHeightStyle = trim,
    )
}

@Composable
fun RText(
    text: String,
    style: TextStyle,
    color: Color,
    modifier: Modifier = Modifier,
    maxLines: Int = Int.MAX_VALUE,
    align: TextAlign? = null,
    ltr: Boolean = false,
    overflow: TextOverflow = TextOverflow.Ellipsis,
) {
    var s = style.copy(color = color)
    if (align != null) s = s.copy(textAlign = align)
    // Addresses and domains are always laid out left-to-right, whatever script surrounds them.
    if (ltr) s = s.copy(textDirection = TextDirection.Ltr)
    BasicText(text = text, modifier = modifier, style = s, maxLines = maxLines, overflow = overflow, softWrap = maxLines != 1)
}
