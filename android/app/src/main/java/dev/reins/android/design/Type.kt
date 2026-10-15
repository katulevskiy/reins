package dev.reins.android.design

import androidx.compose.foundation.layout.BoxWithConstraints
import androidx.compose.foundation.text.BasicText
import androidx.compose.runtime.Composable
import androidx.compose.runtime.remember
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.text.AnnotatedString
import androidx.compose.ui.text.PlatformTextStyle
import androidx.compose.ui.text.SpanStyle
import androidx.compose.ui.text.TextStyle
import androidx.compose.ui.text.buildAnnotatedString
import androidx.compose.ui.text.font.FontStyle
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.rememberTextMeasurer
import androidx.compose.ui.text.style.LineHeightStyle
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.text.style.TextDirection
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.text.withStyle
import androidx.compose.ui.unit.Constraints
import androidx.compose.ui.unit.TextUnit
import androidx.compose.ui.unit.em
import androidx.compose.ui.unit.sp

/**
 * Type, three voices (Reins Signal, `tokens/signal.tokens.json`): Instrument Serif for what you decide on, the sans
 * (Instrument Sans, or Geist) for what you read, Geist Mono in capitals for what the machine says. The named styles are
 * getters, so they follow the Appearance sans.
 */
object RType {
    private val trim = LineHeightStyle(LineHeightStyle.Alignment.Center, LineHeightStyle.Trim.None)

    fun sans(size: Float, weight: FontWeight = FontWeight.Normal, lineHeight: Float? = null): TextStyle = TextStyle(
        fontFamily = Fonts.sans,
        fontWeight = weight.capped(),
        fontSize = size.sp,
        lineHeight = lineHeight?.sp ?: TextUnit.Unspecified,
        platformStyle = PlatformTextStyle(includeFontPadding = false),
        lineHeightStyle = trim,
    )

    /** Geist Mono comes in 400 and 500 only: heavier asks get 500, never a synthesised bold. */
    fun mono(size: Float, weight: FontWeight = FontWeight.Normal): TextStyle = TextStyle(
        fontFamily = Fonts.mono,
        fontWeight = if (weight > FontWeight.Medium) FontWeight.Medium else weight,
        fontSize = size.sp,
        platformStyle = PlatformTextStyle(includeFontPadding = false),
        lineHeightStyle = trim,
    )

    /** Instrument Serif, always 400. [tracking] in em. */
    fun serif(size: Float, lineHeight: Float? = null, tracking: Float = 0f, italic: Boolean = false): TextStyle = TextStyle(
        fontFamily = Fonts.serif,
        fontWeight = FontWeight.Normal,
        fontStyle = if (italic) FontStyle.Italic else FontStyle.Normal,
        fontSize = size.sp,
        lineHeight = lineHeight?.sp ?: TextUnit.Unspecified,
        letterSpacing = if (tracking != 0f) tracking.em else TextUnit.Unspecified,
        platformStyle = PlatformTextStyle(includeFontPadding = false),
        lineHeightStyle = trim,
    )

    // Serif.
    /** The ask on a field, 45/46; the target in italic (see [AskText]). */
    val ask: TextStyle get() = serif(45f, 46f, -0.012f)

    /** The ask on a waiting card, 25/26. */
    val askSmall: TextStyle get() = serif(25f, 26f, -0.01f)

    /** Screen titles, 44/44. */
    val title: TextStyle get() = serif(44f, 44f, -0.01f)

    /** Numbers that are the point, 40 (54 on a grant field). */
    val stat: TextStyle get() = serif(40f, 40f, -0.01f)

    /** Sheet titles, 32/34, with one italic word. */
    val sheetTitle: TextStyle get() = serif(32f, 34f, -0.01f)

    /** One quiet line under a title, italic 24/30, in ink-2. */
    val mood: TextStyle get() = serif(24f, 30f, italic = true)

    // Sans.
    /** Decision buttons, 16.5 / 600 (14.5 on small ones). */
    val button: TextStyle get() = sans(16.5f, FontWeight.SemiBold)

    /** Row titles, 15.5 / 500. */
    val row: TextStyle get() = sans(15.5f, FontWeight.Medium, 20f)

    /** Explanations, 15/22. */
    val body: TextStyle get() = sans(15f, lineHeight = 22f)

    /** Row subtitles, hints, verdict lines, 13/18. */
    val meta: TextStyle get() = sans(13f, lineHeight = 18f)

    /** Tags (Auto, Ready), 11.5 / 600. */
    val tag: TextStyle get() = sans(11.5f, FontWeight.SemiBold)

    // Mono.
    /** Label lines and section labels: 11.5/14, 500, tracked 0.16em, set in capitals (see [LabelText]). */
    val label: TextStyle get() = mono(11.5f, FontWeight.Medium).copy(lineHeight = 14.sp, letterSpacing = 0.16.em)

    /** Branches, accounts, addresses, SHAs, the timer: 13.5. */
    val value: TextStyle get() = mono(13.5f)

    private fun FontWeight.capped(): FontWeight = if (this > FontWeight.SemiBold) FontWeight.SemiBold else this
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

/** A [RType.label] line: mono capitals, tracked. */
@Composable
fun LabelText(text: String, color: Color, modifier: Modifier = Modifier, maxLines: Int = 1) {
    RText(text.uppercase(), RType.label, color, modifier, maxLines = maxLines)
}

/** [text] with the last occurrence of [target] set in italic ("Push 8 commits to *octo/app*"). */
fun askString(text: String, target: String?): AnnotatedString {
    val at = if (target.isNullOrEmpty()) -1 else text.lastIndexOf(target)
    if (at < 0) return AnnotatedString(text)
    return buildAnnotatedString {
        append(text.substring(0, at))
        withStyle(SpanStyle(fontStyle = FontStyle.Italic)) { append(target!!) }
        append(text.substring(at + target!!.length))
    }
}

/**
 * The size an ask is set at: [style]'s own size while it fits in [maxLines] at [width]; else a step smaller each
 * time, never below [minSize] (where it may take more lines). Font scale applies to both ends, as they are in sp.
 */
internal fun askFontSize(lines: (TextUnit) -> Int, size: TextUnit, minSize: TextUnit, maxLines: Int): TextUnit {
    var s = size.value
    while (s > minSize.value) {
        if (lines(s.sp) <= maxLines) return s.sp
        s -= 1f
    }
    return minSize
}

/**
 * An ask: serif, the target in italic, wrapping to [maxLines] (3) before it shrinks, and never shrinking below
 * [minSize] (32 sp): at that size it takes the lines it needs.
 */
@Composable
fun AskText(
    text: String,
    color: Color,
    modifier: Modifier = Modifier,
    target: String? = null,
    style: TextStyle = RType.ask,
    minSize: TextUnit = 32.sp,
    maxLines: Int = 3,
    align: TextAlign? = null,
) {
    val annotated = remember(text, target) { askString(text, target) }
    val measurer = rememberTextMeasurer(cacheSize = 8)
    val density = LocalDensity.current
    BoxWithConstraints(modifier) {
        val width = constraints.maxWidth
        val size = remember(annotated, style, width, density, minSize, maxLines) {
            if (width == Constraints.Infinity) {
                style.fontSize
            } else {
                askFontSize(
                    lines = { s ->
                        measurer.measure(annotated, styleAt(style, s), constraints = Constraints(maxWidth = width), density = density).lineCount
                    },
                    size = style.fontSize,
                    minSize = minSize,
                    maxLines = maxLines,
                )
            }
        }
        var s = styleAt(style, size).copy(color = color)
        if (align != null) s = s.copy(textAlign = align)
        BasicText(annotated, style = s)
    }
}

/** [style] at [size], its line height scaled with it. */
private fun styleAt(style: TextStyle, size: TextUnit): TextStyle {
    if (size == style.fontSize) return style
    val ratio = size.value / style.fontSize.value
    val lh = if (style.lineHeight.isSp) (style.lineHeight.value * ratio).sp else style.lineHeight
    return style.copy(fontSize = size, lineHeight = lh)
}
