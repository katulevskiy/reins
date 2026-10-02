package dev.rewarden.android.design

import androidx.compose.foundation.Canvas
import androidx.compose.foundation.layout.size
import androidx.compose.runtime.Composable
import androidx.compose.runtime.remember
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.StrokeCap
import androidx.compose.ui.graphics.StrokeJoin
import androidx.compose.ui.graphics.asComposePath
import androidx.compose.ui.graphics.drawscope.Fill
import androidx.compose.ui.graphics.drawscope.Stroke
import androidx.compose.ui.graphics.drawscope.scale
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp
import androidx.core.graphics.PathParser

/** UI glyphs drawn from path data on a 24-unit grid with round 1.6-unit strokes: one weight across the app. */
enum class Glyph(val d: String, val fill: Boolean = false) {
    ChevronRight("M9.5 5.5l6.5 6.5-6.5 6.5"),
    ChevronLeft("M14.5 5.5L8 12l6.5 6.5"),
    Close("M6.5 6.5l11 11M17.5 6.5l-11 11"),
    Plus("M12 5v14M5 12h14"),
    Check("M5 12.5l4.5 4.5L19 7.5"),
    Shield("M12 3l7.5 3v5.5c0 4.6-3.1 8.4-7.5 9.5-4.4-1.1-7.5-4.9-7.5-9.5V6z"),
    ShieldCheck("M12 3l7.5 3v5.5c0 4.6-3.1 8.4-7.5 9.5-4.4-1.1-7.5-4.9-7.5-9.5V6zM8.7 12l2.4 2.4 4.2-4.6"),
    Mail("M4 6h16v12H4zM4.5 7l7.5 6 7.5-6"),
    Send("M20.5 3.5L3.5 10.5l6.5 2.5 2.5 6.5zM10 13l4.5-4.5"),
    Search("M10.5 17.5a7 7 0 1 0 0-14 7 7 0 0 0 0 14zM15.8 15.8L20.5 20.5"),
    Clock("M12 20.5a8.5 8.5 0 1 0 0-17 8.5 8.5 0 0 0 0 17zM12 7.5v4.5l3 2"),
    Bell("M6 16v-5a6 6 0 1 1 12 0v5l1.5 2h-15zM10 20.5a2 2 0 0 0 4 0"),
    Phone("M7.5 3h9v18h-9zM11 18h2"),
    Link("M10 14a4 4 0 0 0 5.66 0l3-3a4 4 0 0 0-5.66-5.66l-1 1M14 10a4 4 0 0 0-5.66 0l-3 3a4 4 0 0 0 5.66 5.66l1-1"),
    Warning("M12 4l9 16H3zM12 10v4.5M12 17.25v.25"),
    Gear("M12 15a3 3 0 1 0 0-6 3 3 0 0 0 0 6zM19.4 13.5l1.3 1-1.8 3.1-1.6-.5a7.5 7.5 0 0 1-1.9 1.1l-.3 1.6h-3.6l-.3-1.6a7.5 7.5 0 0 1-1.9-1.1l-1.6.5-1.8-3.1 1.3-1a7.6 7.6 0 0 1 0-2.2l-1.3-1 1.8-3.1 1.6.5a7.5 7.5 0 0 1 1.9-1.1l.3-1.6h3.6l.3 1.6a7.5 7.5 0 0 1 1.9 1.1l1.6-.5 1.8 3.1-1.3 1a7.6 7.6 0 0 1 0 2.2z"),
    Key("M8 15.5a3.5 3.5 0 1 1 3.2-5h9.3v3h-2v2h-3v-2h-4.3A3.5 3.5 0 0 1 8 15.5z"),
    List("M4.5 6.5h15M4.5 11.5h15M4.5 16.5h8"),
    SignOut("M13.5 4.5h-8v15h8M10 12h10.5M17 8.5l3.5 3.5-3.5 3.5"),
    Info("M12 20.5a8.5 8.5 0 1 0 0-17 8.5 8.5 0 0 0 0 17zM12 11v5.5M12 7.75v.25"),
    Trash("M4 7h16M9 7V4.5h6V7M6 7l1 12.5A1.5 1.5 0 0 0 8.5 21h7a1.5 1.5 0 0 0 1.5-1.5L18 7M10 11v6M14 11v6"),
    Refresh("M19.5 12a7.5 7.5 0 1 1-2.2-5.3M19.5 4.5v4.5H15"),
    Tray("M4 13l2.5-8h11L20 13v6H4zM4 13h4.5l1 2h5l1-2H20"),
    Apps("M5 5h5.5v5.5H5zM13.5 5H19v5.5h-5.5zM5 13.5h5.5V19H5zM13.5 13.5H19V19h-5.5z"),
    Download("M12 4v11M7.5 10.5L12 15l4.5-4.5M5 19.5h14"),
    Speaker("M4 9.5h3.5L12 5.5v13l-4.5-4H4zM15.5 9.2a4 4 0 0 1 0 5.6M18.2 6.5a7.8 7.8 0 0 1 0 11"),
    Vibrate("M8.5 4h7v16h-7zM5 9v6M19 9v6M2.5 10.5v3M21.5 10.5v3"),
    Play("M8 5.5v13l10.5-6.5z"),
    Copy("M9 9h10.5v10.5H9zM15 9V4.5H4.5V15H9"),

    /** A QR code to scan (a computer's pairing code). */
    Qr("M4 4h6v6H4zM14 4h6v6h-6zM4 14h6v6H4zM14 14h2v2h-2zM18 18h2v2h-2zM14 18.5h1.5M18.5 14H20v1.5"),

    /** A computer (the desktop app). */
    Laptop("M5.5 6h13v9h-13zM3 18.5h18"),

    // Autopilot.
    /** Manual: your own hand decides. */
    Hand("M8 13V6.5a1.5 1.5 0 0 1 3 0V11M11 11V5a1.5 1.5 0 0 1 3 0v6M14 11V6.5a1.5 1.5 0 0 1 3 0V14.5a6 6 0 0 1-6 6h-.6a5.4 5.4 0 0 1-4.5-2.4L4.6 16a1.5 1.5 0 0 1 2.5-1.7L8 15.5"),

    /** Assisted, and Autopilot's suggestions: a spark beside you. */
    Sparkle("M10.5 4l1.8 4.7 4.7 1.8-4.7 1.8-1.8 4.7-1.8-4.7L4 10.5l4.7-1.8zM17.5 14.5l.9 2.1 2.1.9-2.1.9-.9 2.1-.9-2.1-2.1-.9 2.1-.9z"),

    /** Auto: a heading Autopilot holds by itself. */
    Navigate("M12 3.5l7 17-7-4-7 4z"),

    /** Bypass. */
    Bolt("M13.5 3L5.5 13.5h6L10.5 21l8-10.5h-6z"),
    Lock("M6 10.5h12v10H6zM8.5 10.5V7.5a3.5 3.5 0 0 1 7 0v3M12 14.5v2"),
    Unlock("M6 10.5h12v10H6zM8.5 10.5V7.5a3.5 3.5 0 0 1 6.9-.8M12 14.5v2"),
    Wifi("M3.5 9.5a12 12 0 0 1 17 0M6.5 12.75a7.8 7.8 0 0 1 11 0M9.5 16a3.6 3.6 0 0 1 5 0M12 19.25v.25"),
    Chip("M7 7h10v10H7zM10 10h4v4h-4zM9.5 4v3M14.5 4v3M9.5 17v3M14.5 17v3M4 9.5h3M4 14.5h3M17 9.5h3M17 14.5h3"),
    Stop("M7.5 7.5h9v9h-9z"),
    Pencil("M4.5 19.5l1-4.5L15.5 5l3.5 3.5-10 10zM13.5 7l3.5 3.5"),
    Star("M12 4l2.4 5 5.4.7-4 3.8 1 5.4L12 16.3l-4.8 2.6 1-5.4-4-3.8 5.4-.7z"),
    Flask("M9.5 3.5h5M10.5 3.5v5.5l-5.2 9.4A1.7 1.7 0 0 0 6.8 21h10.4a1.7 1.7 0 0 0 1.5-2.6L13.5 9V3.5M7.6 15h8.8"),
    Flag("M6 21V4M6 4.5h11.5l-2.2 4 2.2 4H6"),
    People("M9 11a3.5 3.5 0 1 0 0-7 3.5 3.5 0 0 0 0 7zM3 20a6 6 0 0 1 12 0M16 4.3a3.5 3.5 0 0 1 0 6.4M18 14.5a6 6 0 0 1 3 5.5");

    val path by lazy { PathParser.createPathFromPathData(d) }
}

@Composable
fun GlyphIcon(glyph: Glyph, tint: Color, size: Dp = 18.dp, modifier: Modifier = Modifier, weight: Float = 1.6f) {
    val path = remember(glyph) { glyph.path.asComposePath() }
    Canvas(modifier.size(size)) {
        val k = this.size.minDimension / 24f
        scale(k, k, pivot = androidx.compose.ui.geometry.Offset.Zero) {
            if (glyph.fill) {
                drawPath(path, tint, style = Fill)
            } else {
                drawPath(path, tint, style = Stroke(width = weight, cap = StrokeCap.Round, join = StrokeJoin.Round))
            }
        }
    }
}
