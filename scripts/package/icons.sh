#!/usr/bin/env bash
# Renders the desktop app's icons from the Reins mark (docs/assets/logo.svg) into crates/rewarden-desktop-app/assets/.
# The PNGs are committed; run this again only when the mark changes. Needs resvg (`cargo install resvg`) or
# rsvg-convert to render SVG, and ImageMagick 7 (`magick`) to compose and to write the .ico.
#
#   app-1024.png, app-256.png  the full-bleed app icon (the window, Linux)
#   app-macos-1024.png         the same mark on the macOS icon grid (824 px plate, transparent margin)
#   ../packaging/windows/Reins.ico   the Windows icon, 16 to 256 px (the .exe resource and the installer)
#   tray-{on,paused,pair}.png         macOS menu bar template images (black on transparent, 36 px = 18 pt @2x)
#   tray-color-{on,paused,pair}.png   Windows and Linux tray icons (64 px)
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
out="$root/crates/rewarden-desktop-app/assets/icons"
command -v magick >/dev/null || {
    echo "icons.sh needs ImageMagick 7 (magick)" >&2
    exit 1
}
mkdir -p "$out"
tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT

# svg FILE SIZE OUT: renders a square SVG at SIZE pixels.
if command -v resvg >/dev/null; then
    svg() { resvg -w "$2" -h "$2" "$1" "$3"; }
elif command -v rsvg-convert >/dev/null; then
    svg() { rsvg-convert -w "$2" -h "$2" -o "$3" "$1"; }
else
    echo "icons.sh needs resvg (cargo install resvg) or rsvg-convert" >&2
    exit 1
fi

# The app icon.
svg "$root/docs/assets/logo.svg" 1024 "$out/app-1024.png"
svg "$root/docs/assets/logo.svg" 256 "$out/app-256.png"
svg "$root/docs/assets/logo.svg" 824 "$tmp/plate.png"
magick -size 1024x1024 xc:none "$tmp/plate.png" -gravity center -composite "$out/app-macos-1024.png"
for s in 16 24 32 48 64 128 256; do svg "$root/docs/assets/logo.svg" "$s" "$tmp/ico-$s.png"; done
magick "$tmp"/ico-{16,24,32,48,64,128,256}.png "$root/crates/rewarden-desktop-app/packaging/windows/Reins.ico"

# The tray glyphs: the shield of the mark with what is inside it saying the state.
shield='M12,3l7.5,3v5.5c0,4.6 -3.1,8.4 -7.5,9.5 -4.4,-1.1 -7.5,-4.9 -7.5,-9.5V6z'
inner_on='<path d="M8.7,12l2.4,2.4 4.2,-4.6" stroke-width="1.9"/>'
inner_paused='<path d="M10,9.3v5.2M14,9.3v5.2" stroke-width="1.9"/>'
inner_pair='<path d="M12,8.2v4.6" stroke-width="1.9"/><circle cx="12" cy="16" r="1.1" fill="currentColor" stroke="none"/>'

template() { # name inner
    cat >"$tmp/$1.svg" <<SVG
<svg xmlns="http://www.w3.org/2000/svg" viewBox="1.5 1.5 21 21" width="21" height="21">
  <g fill="none" stroke="#000" color="#000" stroke-linecap="round" stroke-linejoin="round">
    <path d="$shield" stroke-width="1.7"/>$2
  </g>
</svg>
SVG
    svg "$tmp/$1.svg" 36 "$out/tray-$1.png"
}
template on "$inner_on"
template paused "$inner_paused"
template pair "$inner_pair"

color() { # name inner glyph-colour
    cat >"$tmp/c-$1.svg" <<SVG
<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 108 108" width="108" height="108">
  <rect width="108" height="108" rx="24" fill="#170B33"/>
  <g transform="translate(27 27) scale(2.25)" fill="none" stroke-linecap="round" stroke-linejoin="round">
    <path d="$shield" stroke="#C9C1FB" stroke-width="1.6"/>
    <g stroke="$3" color="$3">$2</g>
  </g>
</svg>
SVG
    svg "$tmp/c-$1.svg" 64 "$out/tray-color-$1.png"
}
color on "$inner_on" "#FFFFFF"
color paused "$inner_paused" "#FACC15"
color pair "$inner_pair" "#F87171"

# The disk image's window: Reins on the left, Applications on the right (scripts/package/macos.sh puts the icons at
# 165,170 and 495,170 of a 660x400 background), an arrow between them and one line of text.
fonts="$root/ios/Shared/Fonts"
dmg_background() { # scale out
    s=$1
    cat >"$tmp/dmg.svg" <<SVG
<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 660 400" width="$((660 * s))" height="$((400 * s))">
  <defs>
    <linearGradient id="g" x1="0" y1="0" x2="0" y2="1">
      <stop offset="0" stop-color="#F7F6FD"/><stop offset="1" stop-color="#ECEAF8"/>
    </linearGradient>
  </defs>
  <rect width="660" height="400" fill="url(#g)"/>
  <path d="M262,170 H388 M372,156 L390,170 L372,184" fill="none" stroke="#5B43E8" stroke-width="3.5"
        stroke-linecap="round" stroke-linejoin="round" opacity="0.85"/>
  <text x="330" y="286" text-anchor="middle" font-family="Geist" font-weight="500" font-size="15" fill="#27272C">Drag Reins to Applications</text>
  <text x="330" y="308" text-anchor="middle" font-family="Geist" font-size="12" fill="#62626A">Then open it from there: it lives in your menu bar.</text>
</svg>
SVG
    if command -v resvg >/dev/null; then
        resvg --use-fonts-dir "$fonts" "$tmp/dmg.svg" "$2"
    else
        rsvg-convert -o "$2" "$tmp/dmg.svg"
    fi
}
dmg_background 1 "$out/dmg-background.png"
dmg_background 2 "$out/dmg-background@2x.png"

ls -l "$out"
