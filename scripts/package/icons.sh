#!/usr/bin/env bash
# Renders the disk image background (dmg-background.png, dmg-background@2x.png) into
# crates/reins-desktop-app/assets/icons/. The PNGs are committed; run this again only when it changes. Needs resvg
# (`cargo install resvg`) or rsvg-convert to render SVG.
#
# The app icons, the .ico and every tray icon come from scripts/brand/render-logo.sh.
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
out="$root/crates/reins-desktop-app/assets/icons"
mkdir -p "$out"
tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT

command -v resvg >/dev/null || command -v rsvg-convert >/dev/null || {
    echo "icons.sh needs resvg (cargo install resvg) or rsvg-convert" >&2
    exit 1
}

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
