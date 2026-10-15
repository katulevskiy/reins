#!/usr/bin/env python3
"""Generates the Android `Glyph` enum from the Signal glyph set.

Reads every SVG in design/signal/assets/icons (24 grid, 1.6 stroke, round caps and joins, currentColor) and writes
android/app/src/main/java/dev/reins/android/design/Glyphs.kt: one entry per glyph, its shapes as path data (circle,
ellipse, rect, line, polyline and polygon become paths), and which of them are filled (even-odd where the SVG says
so, for knockouts), stroked, or drawn white.

Names the app used before Signal stay compiling: where the set has a glyph with the same meaning they are aliases of
it (`Glyph.Clock` is `Glyph.Timer`); where it has none yet they keep their old path data until one is drawn into the
set (then the alias takes over on the next run).

Usage: python3 android/scripts/glyphs.py   (from anywhere; paths are relative to the repo)
"""

import pathlib
import re
import sys
import xml.etree.ElementTree as ET

ROOT = pathlib.Path(__file__).resolve().parents[2]
ICONS = ROOT / "design/signal/assets/icons"
OUT = ROOT / "android/app/src/main/java/dev/reins/android/design/Glyphs.kt"
SVG = "{http://www.w3.org/2000/svg}"
DEFAULT_WIDTH = 1.6

# Old name -> the Signal glyph with the same meaning.
ALIASES = {
    "Clock": "timer",
    "SignOut": "quit",
    "Qr": "qr-code",
    "Hand": "person",  # Manual: the design draws it as a person.
    "Bolt": "autopilot",  # Auto and Bypass: the bolt.
    "Sparkle": "autopilot",  # Autopilot's suggestions.
    "Vibrate": "haptics",
}

# Old names the set has no glyph for yet: (the name it will have in the set, the old path data). When the set gains
# the glyph the old name becomes an alias of it.
LEGACY = {
    "List": ("list", "M4.5 6.5h15M4.5 11.5h15M4.5 16.5h8"),
    "Info": ("info", "M12 20.5a8.5 8.5 0 1 0 0-17 8.5 8.5 0 0 0 0 17zM12 11v5.5M12 7.75v.25"),
    "Tray": ("tray", "M4 13l2.5-8h11L20 13v6H4zM4 13h4.5l1 2h5l1-2H20"),
    "Apps": ("apps", "M5 5h5.5v5.5H5zM13.5 5H19v5.5h-5.5zM5 13.5h5.5V19H5zM13.5 13.5H19V19h-5.5z"),
    "Speaker": ("speaker", "M4 9.5h3.5L12 5.5v13l-4.5-4H4zM15.5 9.2a4 4 0 0 1 0 5.6M18.2 6.5a7.8 7.8 0 0 1 0 11"),
    "Bag": ("bag", "M5.5 8h13l-1 12.5h-11zM9 10V7a3 3 0 0 1 6 0v3"),
    "Unlock": ("lock-open", "M6 10.5h12v10H6zM8.5 10.5V7.5a3.5 3.5 0 0 1 6.9-.8M12 14.5v2"),
    "Wifi": ("wifi", "M3.5 9.5a12 12 0 0 1 17 0M6.5 12.75a7.8 7.8 0 0 1 11 0M9.5 16a3.6 3.6 0 0 1 5 0M12 19.25v.25"),
    "Flag": ("flag", "M6 21V4M6 4.5h11.5l-2.2 4 2.2 4H6"),
    "People": ("people", "M9 11a3.5 3.5 0 1 0 0-7 3.5 3.5 0 0 0 0 7zM3 20a6 6 0 0 1 12 0M16 4.3a3.5 3.5 0 0 1 0 6.4M18 14.5a6 6 0 0 1 3 5.5"),
}


def num(v: float) -> str:
    s = f"{v:.3f}".rstrip("0").rstrip(".")
    return "0" if s in ("-0", "") else s


def f(el, name, default=0.0) -> float:
    v = el.get(name)
    return float(v) if v is not None else default


def style(el) -> dict:
    out = {}
    for decl in (el.get("style") or "").split(";"):
        if ":" in decl:
            k, v = decl.split(":", 1)
            out[k.strip()] = v.strip()
    return out


def ellipse_d(cx, cy, rx, ry) -> str:
    return f"M{num(cx - rx)} {num(cy)}a{num(rx)} {num(ry)} 0 1 0 {num(2 * rx)} 0a{num(rx)} {num(ry)} 0 1 0 {num(-2 * rx)} 0z"


def rect_d(x, y, w, h, rx, ry) -> str:
    if rx <= 0 and ry <= 0:
        return f"M{num(x)} {num(y)}h{num(w)}v{num(h)}h{num(-w)}z"
    rx = min(rx or ry, w / 2)
    ry = min(ry or rx, h / 2)
    return (
        f"M{num(x + rx)} {num(y)}h{num(w - 2 * rx)}a{num(rx)} {num(ry)} 0 0 1 {num(rx)} {num(ry)}"
        f"v{num(h - 2 * ry)}a{num(rx)} {num(ry)} 0 0 1 {num(-rx)} {num(ry)}"
        f"h{num(-(w - 2 * rx))}a{num(rx)} {num(ry)} 0 0 1 {num(-rx)} {num(-ry)}"
        f"v{num(-(h - 2 * ry))}a{num(rx)} {num(ry)} 0 0 1 {num(rx)} {num(-ry)}z"
    )


def shape_d(el) -> str:
    tag = el.tag.replace(SVG, "")
    if tag == "path":
        return " ".join(el.get("d").split())
    if tag == "circle":
        r = f(el, "r")
        return ellipse_d(f(el, "cx"), f(el, "cy"), r, r)
    if tag == "ellipse":
        return ellipse_d(f(el, "cx"), f(el, "cy"), f(el, "rx"), f(el, "ry"))
    if tag == "rect":
        rx, ry = el.get("rx"), el.get("ry")
        rxv = float(rx) if rx is not None else (float(ry) if ry is not None else 0.0)
        ryv = float(ry) if ry is not None else rxv
        return rect_d(f(el, "x"), f(el, "y"), f(el, "width"), f(el, "height"), rxv, ryv)
    if tag == "line":
        return f"M{num(f(el, 'x1'))} {num(f(el, 'y1'))}L{num(f(el, 'x2'))} {num(f(el, 'y2'))}"
    if tag in ("polyline", "polygon"):
        pts = [float(p) for p in re.split(r"[\s,]+", el.get("points").strip())]
        d = "M" + " ".join(f"{num(pts[i])} {num(pts[i + 1])}" for i in range(0, len(pts), 2))
        return d + ("z" if tag == "polygon" else "")
    raise SystemExit(f"unsupported SVG element <{tag}>")


def paint(value) -> str:
    """'none', 'current' or 'white'."""
    if value is None or value == "none":
        return "none"
    v = value.lower()
    if v == "currentcolor":
        return "current"
    if v in ("#fff", "#ffffff", "white"):
        return "white"
    raise SystemExit(f"unsupported paint {value!r}")


def parts(svg_path: pathlib.Path) -> list:
    root = ET.parse(svg_path).getroot()
    base_fill = root.get("fill", "none")
    base_stroke = root.get("stroke", "none")
    base_width = float(root.get("stroke-width", DEFAULT_WIDTH))
    out = []
    for el in root.iter():
        tag = el.tag.replace(SVG, "")
        if tag in ("svg", "g", "title", "desc", "defs"):
            continue
        st = style(el)
        fill = paint(st.get("fill", el.get("fill", base_fill)))
        stroke = paint(st.get("stroke", el.get("stroke", base_stroke)))
        width = float(st.get("stroke-width", el.get("stroke-width", base_width)))
        even_odd = st.get("fill-rule", el.get("fill-rule", root.get("fill-rule", "nonzero"))) == "evenodd"
        out.append((shape_d(el), fill, stroke, width, even_odd))
    return out


def pascal(name: str) -> str:
    return "".join(p[:1].upper() + p[1:] for p in re.split(r"[-_]", name))


def kotlin_part(d, fill, stroke, width, even_odd) -> str:
    args = [f'"{d}"']
    if fill != "none":
        args.append("fill = true")
    if stroke == "none":
        args.append("stroke = false")
    if "white" in (fill, stroke):
        args.append("white = true")
    if even_odd:
        args.append("evenOdd = true")
    if abs(width - DEFAULT_WIDTH) > 1e-6:
        args.append(f"width = {num(width)}f")
    return f"GlyphPart({', '.join(args)})"


def main() -> None:
    svgs = sorted(p for p in ICONS.glob("*.svg"))
    if not svgs:
        sys.exit(f"no SVGs in {ICONS}")
    names = {p.stem for p in svgs}
    entries = []
    for p in svgs:
        ps = parts(p)
        body = ", ".join(kotlin_part(*x) for x in ps)
        entries.append(f"    {pascal(p.stem)}({body}),")
    aliases = dict(ALIASES)
    legacy = []
    for old, (new, d) in LEGACY.items():
        if new in names:
            if pascal(new) != old:  # same name: the new glyph simply replaces the old one
                aliases[old] = new
        else:
            legacy.append(f"    /** Not in the Signal set yet: the glyph from before, until `{new}` is drawn. */\n    {old}(GlyphPart(\"{d}\")),")
    for old, new in aliases.items():
        if new not in names:
            sys.exit(f"alias {old} -> {new}: no {new}.svg")
    alias_lines = "\n\n".join(
        f'        @Deprecated("Signal: {pascal(new)}", ReplaceWith("Glyph.{pascal(new)}"))\n        val {old}: Glyph get() = {pascal(new)}'
        for old, new in sorted(aliases.items())
    )
    text = f"""package dev.reins.android.design

// Generated by android/scripts/glyphs.py from design/signal/assets/icons. Do not edit; run the script.

/**
 * The Reins Signal glyphs: 24 grid, 1.6 stroke, round caps and joins, one weight. Draw with [GlyphIcon], or use
 * [vector] where an [androidx.compose.ui.graphics.vector.ImageVector] is wanted.
 */
enum class Glyph(vararg parts: GlyphPart) {{
{chr(10).join(entries)}

    // Names from before Signal that the set has no glyph for yet.
{chr(10).join(legacy)}
    ;

    val parts: List<GlyphPart> = parts.toList()

    /** As an image vector (every part in the tint; a white part takes the tint too). */
    val vector: androidx.compose.ui.graphics.vector.ImageVector by lazy {{ glyphVector(this) }}

    companion object {{
{alias_lines}
    }}
}}
"""
    OUT.write_text(text)
    print(f"{OUT.relative_to(ROOT)}: {len(svgs)} glyphs, {len(legacy)} legacy, {len(aliases)} aliases")


if __name__ == "__main__":
    main()
