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
 * Type (Reins Next, the Type board): Sora for everything you read and decide on, Geologica under it for Cyrillic, and
 * JetBrains Mono for what the machine says. Display text (titles, asks, numbers) is Sora 500 at 0.74 of its board
 * size, tracked tighter as it grows; the emphasised words of an ask are Sora 600, never italic. The named styles are
 * getters, so they follow the font family once it has loaded.
 */
object RType {
    private val trim = LineHeightStyle(LineHeightStyle.Alignment.Center, LineHeightStyle.Trim.None)

    /** The boards draw display text at `--s × 0.74`. */
    const val DISPLAY_SCALE = 0.74f

    fun sans(size: Float, weight: FontWeight = FontWeight.Normal, lineHeight: Float? = null, tracking: Float = 0f): TextStyle = TextStyle(
        fontFamily = Fonts.sans,
        fontWeight = weight.capped(),
        fontSize = size.sp,
        lineHeight = lineHeight?.sp ?: TextUnit.Unspecified,
        letterSpacing = if (tracking != 0f) tracking.em else TextUnit.Unspecified,
        platformStyle = PlatformTextStyle(includeFontPadding = false),
        lineHeightStyle = trim,
    )

    /** JetBrains Mono comes in 400 and 500 only: heavier text gets 500, never a synthesised bold. */
    fun mono(size: Float, weight: FontWeight = FontWeight.Normal): TextStyle = TextStyle(
        fontFamily = Fonts.mono,
        fontWeight = if (weight > FontWeight.Medium) FontWeight.Medium else weight,
        fontSize = size.sp,
        platformStyle = PlatformTextStyle(includeFontPadding = false),
        lineHeightStyle = trim,
    )

    /**
     * Display text at board size [s] (`.d`, `--s`): Sora 500 at `s × 0.74`, line height 1.04, tracked −0.03 em from
     * s = 40 up (none at 16, linear between).
     */
    fun display(s: Float, weight: FontWeight = FontWeight.Medium): TextStyle {
        val size = s * DISPLAY_SCALE
        val track = -0.03f * ((s - 16f) / 24f).coerceIn(0f, 1f)
        return sans(size, weight, lineHeight = size * 1.04f, tracking = track)
    }

    /** A style from before Next, at the Signal serif's [size]: now [display] at that board size. */
    @Suppress("UNUSED_PARAMETER")
    fun serif(size: Float, lineHeight: Float? = null, tracking: Float = 0f, italic: Boolean = false): TextStyle = display(size)

    // Display.
    /** The ask on a field (`--s 45`, 33). */
    val ask: TextStyle get() = display(45f)

    /** The ask on a waiting card (`--s 30`, 22). */
    val askSmall: TextStyle get() = display(30f)

    /** Tab page titles (`--s 40`, 30). */
    val title: TextStyle get() = display(40f)

    /** Pushed page titles (`--s 32`, 24). */
    val pageTitle: TextStyle get() = display(32f)

    /** Numbers that are the point (`--s 64`). */
    val stat: TextStyle get() = display(64f)

    /** Sheet titles (`--s 30`, 22). */
    val sheetTitle: TextStyle get() = display(30f)

    /** Kept for callers from before Next: Next has no mood line, so it reads as body. */
    val mood: TextStyle get() = sans(17f, lineHeight = 23f)

    // Sans.
    /** Decision buttons, 16.5 / 600 (15 on small ones). */
    val button: TextStyle get() = sans(16.5f, FontWeight.SemiBold)

    /** Row titles, 16 / 500. */
    val row: TextStyle get() = sans(16f, FontWeight.Medium, 21.6f)

    /** Row sentences (Activity), 16 / 400. */
    val rowSentence: TextStyle get() = sans(16f, lineHeight = 21.6f)

    /** Explanations, 15/22. */
    val body: TextStyle get() = sans(15f, lineHeight = 22f)

    /** Row subtitles, hints, times, 12.5 / 17. */
    val meta: TextStyle get() = sans(12.5f, lineHeight = 17f, tracking = 0.01f)

    /** Tags (Auto, Ready), 11.5 / 600. */
    val tag: TextStyle get() = sans(11.5f, FontWeight.SemiBold)

    /** Section headers (`.sec`): 13 / 600, tracked 0.015 em, sentence case. */
    val section: TextStyle get() = sans(13f, FontWeight.SemiBold, tracking = 0.015f)

    // Mono.
    /** Kept for callers from before Next: the section header. */
    val label: TextStyle get() = section

    /** Branches, accounts, addresses, SHAs, the timer: 13. */
    val value: TextStyle get() = mono(13f)

    /** Code and diffs, 11.5. */
    val code: TextStyle get() = mono(11.5f)

    /** Codes you compare (pairing, join): 24 mono 500. */
    val codes: TextStyle get() = mono(24f, FontWeight.Medium)

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

/** A section header line ([RType.section]): sentence case, as written. */
@Composable
fun LabelText(text: String, color: Color, modifier: Modifier = Modifier, maxLines: Int = 1) {
    RText(text, RType.section, color, modifier, maxLines = maxLines)
}

/** [text] with the last occurrence of [target] emphasised: Sora 600 ("Push 8 commits to **octo/app**"). */
fun askString(text: String, target: String?): AnnotatedString {
    val at = if (target.isNullOrEmpty()) -1 else text.lastIndexOf(target)
    if (at < 0) return AnnotatedString(text)
    return buildAnnotatedString {
        append(text.substring(0, at))
        withStyle(SpanStyle(fontWeight = FontWeight.SemiBold)) { append(target!!) }
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
 * An ask: display type, the target emphasised, wrapping to [maxLines] (3) before it shrinks, and never shrinking
 * below [minSize] (24 sp): at that size it takes the lines it needs.
 */
@Composable
fun AskText(
    text: String,
    color: Color,
    modifier: Modifier = Modifier,
    target: String? = null,
    style: TextStyle = RType.ask,
    minSize: TextUnit = 24.sp,
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
