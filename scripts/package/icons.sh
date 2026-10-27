#!/usr/bin/env bash
# Renders the desktop app's status icons and the disk image background into crates/reins-desktop-app/assets/icons/.
# The PNGs are committed; run this again only when they change. Needs resvg (`cargo install resvg`) or rsvg-convert
# to render SVG, and ImageMagick 7 (`magick`).
#
#   tray-{waiting,paused,pair}.png, tray-color-{waiting,paused,pair}.png   the tray's status shields (below)
#   dmg-background.png, dmg-background@2x.png                               the disk image's window
#
# The Reins logo itself (the app icons, the .ico, the macOS template and the mode icons in the tray) comes from
# scripts/brand/render-logo.sh.
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
out="$root/crates/reins-desktop-app/assets/icons"
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

# The tray's status shields, from the Signal glyphs (assets/icons/glyphs, `Desk-Extras.png` "Shield states"): on the
# field's colour when something needs you: push orange while a request waits, delete red while git is paused, warn
# amber when Reins needs the user. (While Reins is simply on, or a mode runs, the tray shows the R instead.)
#   tray-{waiting,paused,pair}.png         macOS, drawn as they are (not templates: they keep their colour), 36 px
#   tray-color-{waiting,paused,pair}.png   Windows and Linux, 64 px
glyphs="$root/crates/reins-desktop-app/assets/icons/glyphs"

# glyph_svg GLYPH COLOUR: the glyph's drawing with its strokes in COLOUR.
glyph_svg() {
    sed -e 's/currentColor/'"$2"'/g' -e 's/<svg [^>]*>//' -e 's#</svg>##' "$glyphs/$1.svg"
}

# plate NAME GLYPH FIELD SIZE OUT: the glyph in white on a rounded plate of FIELD.
plate() {
    cat >"$tmp/p-$1-$4.svg" <<SVG
<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" width="24" height="24">
  <rect x="1" y="1" width="22" height="22" rx="5.5" fill="$3"/>
  <g transform="translate(4.2 4.2) scale(0.65)" fill="none" stroke="#FFFFFF" stroke-width="2.2" stroke-linecap="round"
     stroke-linejoin="round">$(glyph_svg "$2" '#FFFFFF' | sed 's/stroke-width="[0-9.]*"//g')</g>
</svg>
SVG
    svg "$tmp/p-$1-$4.svg" "$4" "$5"
}
for state in "waiting shield-waiting #C2410C" "paused shield-paused #B91C1C" "pair shield-alert #A16207"; do
    set -- $state
    plate "$1" "$2" "$3" 64 "$out/tray-color-$1.png"
    plate "$1" "$2" "$3" 36 "$out/tray-$1.png"
done

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
