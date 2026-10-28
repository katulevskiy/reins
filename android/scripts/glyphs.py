#!/usr/bin/env python3
"""Generates the Android `Glyph` enum from Material Symbols (Rounded, weight 400, fill 0).

Reins Next draws every glyph from Material Symbols on Android (design/next/PLAN.md). The app names its glyphs by
meaning (`Glyph.Autopilot`, `Glyph.Bypass`...); MAP below says which symbol each one is. The symbols come from the
`@material-symbols/svg-400` npm package (Apache-2.0, https://github.com/marella/material-symbols), on the 960 grid
(viewBox 0 -960 960 960); `GlyphPart` scales them to the 24 grid when it parses them.

Usage: python3 android/scripts/glyphs.py <path to the package's rounded/ directory>
       (for example node_modules/@material-symbols/svg-400/rounded, or an unpacked tarball of the package)
"""

import pathlib
import sys
import xml.etree.ElementTree as ET

ROOT = pathlib.Path(__file__).resolve().parents[2]
OUT = ROOT / "android/app/src/main/java/dev/reins/android/design/Glyphs.kt"
SVG = "{http://www.w3.org/2000/svg}"

# Glyph name -> Material Symbol (Rounded). "-fill" picks the filled variant. One glyph, one meaning (System board,
# Icon vocabulary): the bolt is only Autopilot, Bypass is a shield with a mark, Lockdown is a block.
MAP = {
    "Activity": "vital_signs",
    "Appearance": "contrast",
    "ArrowBack": "arrow_back",
    "ArrowForward": "arrow_forward",
    "AutopilotOff": "flash_off",
    "Autopilot": "bolt",
    "AutopilotFilled": "bolt-fill",
    "Bell": "notifications",
    "Block": "block",
    "Briefcase": "work",
    "Bypass": "gpp_maybe",
    "Calendar": "calendar_today",
    "Camera": "photo_camera",
    "Card": "credit_card",
    "Check": "check",
    "ChevronDown": "keyboard_arrow_down",
    "ArrowDown": "arrow_downward",
    "ChevronLeft": "chevron_left",
    "ChevronRight": "chevron_right",
    "ChevronUp": "keyboard_arrow_up",
    "Chip": "memory",
    "CircleAlert": "error",
    "CircleCheck": "check_circle",
    "CircleClose": "cancel",
    "Clipboard": "content_paste",
    "Close": "close",
    "CloudCog": "cloud_sync",
    "CloudUpload": "cloud_upload",
    "Cloud": "cloud",
    "Copy": "content_copy",
    "Database": "database",
    "Devices": "devices",
    "Download": "download",
    "Eye": "visibility",
    "FaceId": "face",
    "Family": "family_restroom",
    "FileCode": "code_blocks",
    "FileLock": "encrypted",
    "FileText": "description",
    "File": "draft",
    "Filter": "filter_list",
    "Fingerprint": "fingerprint",
    "Flask": "science",
    "FolderSearch": "folder_open",
    "GitBranch": "alt_route",
    "GitCommit": "commit",
    "Globe": "language",
    "Haptics": "mobile_vibrate",
    "HardDrive": "hard_drive",
    "Heart": "favorite",
    "Help": "help",
    "Home": "home",
    "IdCard": "id_card",
    "Infinity": "all_inclusive",
    "Key": "key",
    "Laptop": "laptop_mac",
    "Lifebuoy": "support",
    "Link": "link",
    "LockOpen": "lock_open",
    "Lock": "lock",
    "Mail": "mail",
    "Minus": "remove",
    "Moon": "dark_mode",
    "More": "more_horiz",
    "Package": "package_2",
    "Pause": "pause",
    "Pencil": "edit",
    "Person": "person",
    "Phone": "mobile",
    "Play": "play_arrow",
    "Plus": "add",
    "Power": "power_settings_new",
    "QrCode": "qr_code_scanner",
    "Quit": "logout",
    "Refresh": "refresh",
    "Rocket": "rocket_launch",
    "ScanSearch": "document_scanner",
    "Search": "search",
    "Send": "send",
    "Server": "dns",
    "Settings": "tune",
    "Share": "ios_share",
    "ShieldAlert": "gpp_maybe",
    "ShieldCheckFilled": "verified_user-fill",
    "ShieldCheck": "verified_user",
    "ShieldPaused": "shield_lock",
    "ShieldWaiting": "shield_question",
    "Shield": "shield",
    "Speaker": "volume_up",
    "Star": "star",
    "StarFilled": "star-fill",
    "Terminal": "terminal",
    "TimerOff": "timer_off",
    "Timer": "timer",
    "Token": "confirmation_number",
    "Trash": "delete",
    "Tray": "inbox",
    "Undo": "undo",
    "Unlink": "link_off",
    "Vault": "encrypted",
    "Warning": "warning",
    "List": "list",
    "Info": "info",
    "Apps": "apps",
    "Bag": "shopping_bag",
    "Wifi": "wifi",
    "Flag": "flag",
    "People": "group",
    "Passkey": "passkey",
    "Photo": "add_photo_alternate",
    "Sync": "sync",
    "Robot": "smart_toy",
}

# Names from before Next that keep compiling as aliases.
ALIASES = {
    "Bolt": "Autopilot",
    "Clock": "Timer",
    "Hand": "Person",
    "Qr": "QrCode",
    "SignOut": "Quit",
    "Sparkle": "Autopilot",
    "Unlock": "LockOpen",
    "Vibrate": "Haptics",
}


def path_data(svg: pathlib.Path) -> list:
    root = ET.parse(svg).getroot()
    if root.get("viewBox") != "0 -960 960 960":
        sys.exit(f"{svg.name}: unexpected viewBox {root.get('viewBox')}")
    out = []
    for el in root.iter():
        tag = el.tag.replace(SVG, "")
        if tag in ("svg", "g", "title", "desc", "defs"):
            continue
        if tag != "path":
            sys.exit(f"{svg.name}: unsupported <{tag}>")
        out.append(" ".join(el.get("d").split()))
    return out


def main() -> None:
    if len(sys.argv) != 2:
        sys.exit(__doc__)
    src = pathlib.Path(sys.argv[1])
    entries = []
    for name, symbol in MAP.items():
        f = src / f"{symbol}.svg"
        if not f.is_file():
            sys.exit(f"no {f}")
        parts = ", ".join(f'GlyphPart("{d}")' for d in path_data(f))
        entries.append(f"    /** Material Symbols `{symbol}`. */\n    {name}({parts}),")
    alias_lines = "\n\n".join(
        f'        @Deprecated("Next: {new}", ReplaceWith("Glyph.{new}"))\n        val {old}: Glyph get() = {new}'
        for old, new in sorted(ALIASES.items())
    )
    text = f"""package dev.reins.android.design

// Generated by android/scripts/glyphs.py from Material Symbols (Rounded, 400, fill 0; Apache-2.0,
// https://github.com/marella/material-symbols). Do not edit; run the script.

/**
 * The Reins Next glyphs: Material Symbols Rounded on their 960 grid, filled shapes; [GlyphPart] scales them to the
 * 24 grid. Draw with [GlyphIcon], or use [vector] where an [androidx.compose.ui.graphics.vector.ImageVector] is
 * wanted.
 */
enum class Glyph(vararg parts: GlyphPart) {{
{chr(10).join(entries)}
    ;

    val parts: List<GlyphPart> = parts.toList()

    /** As an image vector, to be tinted. */
    val vector: androidx.compose.ui.graphics.vector.ImageVector by lazy {{ glyphVector(this) }}

    companion object {{
{alias_lines}
    }}
}}
"""
    OUT.write_text(text)
    print(f"wrote {OUT} ({len(entries)} glyphs)")


if __name__ == "__main__":
    main()
