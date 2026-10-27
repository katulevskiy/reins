#!/usr/bin/env bash
# Renders every Reins logo asset from the one source, design/brand/reins-origami-r.svg (the origami R: cream paper,
# navy folds, an orange tip, on black textured paper). The outputs are committed; run this again only when the source
# changes. Needs python3, rsvg-convert (librsvg) and ImageMagick 7 (`magick`).
#
#   scripts/brand/render-logo.sh                      # every asset
#   scripts/brand/render-logo.sh --sheet out.png      # and a contact sheet of the variants at real sizes
#
# The variants (design/brand/, derived from the source, committed so other files can point at them):
#   reins-mark.svg               the R alone on transparent, flat (no texture, no grain): for dark backgrounds
#   reins-mark-light.svg         the same with a navy paper edge, for light backgrounds (48 px and up)
#   reins-mark-light-small.svg   a heavier edge for 16 to 32 px on light backgrounds
#   reins-silhouette.svg         the R in one colour (themed icons, the notification icon, the menu bar)
#   reins-icon-flat.svg          the R on plain black, square: the app icon where it is too small for the texture
# The texture stays only where the icon is large (128 px and up); smaller icons use the flat one.
#
# Written:
#   iOS      ios/Reins/Assets.xcassets/AppIcon.appiconset/AppIcon.png        1024, no alpha (Xcode makes the sizes)
#            ios/Reins/Assets.xcassets/ReinsMark.imageset                    the R, light and dark, 56 pt @1-3x
#            ios/Reins/Assets.xcassets/ReinsIcon.imageset                    the small app icon (flat), 60 pt @1-3x
#   Android  res/mipmap-*/ic_launcher_background.png                         the black texture, 108 dp
#            res/mipmap-*/ic_launcher_foreground.png                         the R inside the 66 dp safe circle
#            res/drawable/ic_launcher_monochrome.xml                         the silhouette, same place (themed icons)
#            res/drawable/ic_notification.xml                                the silhouette on a 24 dp grid
#            play/graphics/icon.svg, icon.png                                Play's 512 icon, full bleed, 32-bit
#   Desktop  assets/icons/app-1024.png, app-256.png                          the window and Linux icon (rounded)
#            assets/icons/app-macos-1024.png                                 824 px plate on the macOS grid (-> .icns)
#            packaging/windows/Reins.ico                                     16 to 256 px
#            assets/icons/logos/reins.svg                                    the in-app mark (flat icon, vector)
#            assets/icons/tray-on.png                                        the macOS template: the black R, 36 px
#            assets/icons/tray-{bypass,overnight,lockdown}.png               macOS, the white R on the mode's colour
#            assets/icons/tray-{waiting,paused,pair}.png                     macOS, the R with the state's badge
#            assets/icons/tray-color-{on,bypass,...,pair}.png                Windows and Linux, 64 px
#   Docs     docs/assets/logo.svg, logo.png                                  the README mark (rounded, 256 px)
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
brand="$root/design/brand"
src="$brand/reins-origami-r.svg"
sheet=""
while (($#)); do
    case "$1" in
    --sheet)
        sheet="$2"
        shift 2
        ;;
    *)
        echo "unknown argument: $1 (--sheet FILE)" >&2
        exit 2
        ;;
    esac
done
for tool in python3 rsvg-convert magick; do
    command -v "$tool" >/dev/null || {
        echo "render-logo.sh needs $tool" >&2
        exit 1
    }
done
mkdir -p "$root/target"
work="$(mktemp -d -p "$root/target" brand-logo.XXXXXX)"
trap 'rm -rf "$work"' EXIT

# ---- the SVG variants ------------------------------------------------------------------------------------------------
python3 - "$src" "$brand" "$work" "$root" <<'PY'
import math
import re
import sys
from pathlib import Path

src_path, brand, work, root = (Path(a) for a in sys.argv[1:])
src = src_path.read_text()
P = "reins-black-wave-large-r-"  # the source's id prefix

# The R: its gradients and filters, its group (faces, folds, tip), its silhouette and where the source puts it.
defs = re.search(r"<defs>(.*?)</defs>", src, re.S).group(1)
r_defs = "".join(
    m.group(0).strip()
    for m in re.finditer(r'<(linearGradient|filter)\s+id="' + P + r'r-[^"]*".*?</\1>', defs, re.S)
)
place = 'translate(627 627) scale(1.44) translate(-640 -629.5)'
assert place in src, "the source no longer places the R where this script expects"
r_group = re.search(r'(<g id="' + P + r'r-logo".*?</g>)\s*</g>\s*</g>\s*</svg>', src, re.S).group(1)
r_flat = r_group.replace(f' filter="url(#{P}r-depth)"', "").replace(f' filter="url(#{P}r-paperGrain)"', "")
sil = re.search(r'id="' + P + r'r-silhouette" d="([^"]*)"', src).group(1)

# The R's box (from the silhouette, which includes the orange tip) and the circle around its centre.
nums = [float(n) for n in re.findall(r"-?\d+(?:\.\d+)?", sil)]
xs, ys = nums[0::2], nums[1::2]
x0, x1, y0, y1 = min(xs), max(xs), min(ys), max(ys)
cx, cy = (x0 + x1) / 2, (y0 + y1) / 2
# The corners are rounded: the outer control points overstate the radius by about 2 %.
radius = max(math.hypot(x - cx, y - cy) for x, y in zip(xs, ys)) * 0.98
side = 830  # the marks' square: the R and its light edge, a little air
vb = f"{cx - side / 2:g} {cy - side / 2:g} {side} {side}"

def svg(view, body, extra_defs="", size=None):
    w = size or view.split()[2]
    return (f'<svg xmlns="http://www.w3.org/2000/svg" viewBox="{view}" width="{w}" height="{w}" role="img" '
            f'aria-label="Reins">\n<defs>{r_defs}{extra_defs}</defs>\n{body}\n</svg>\n')

def edge(width):
    # A navy paper edge behind the faces: only its outer half shows.
    return (f'<path d="{sil}" fill="none" stroke="#0c2540" stroke-width="{width}" stroke-linejoin="round"/>')

out = {
    "reins-mark.svg": svg(vb, r_flat),
    "reins-mark-light.svg": svg(vb, edge(20) + r_flat),
    "reins-mark-light-small.svg": svg(vb, edge(64) + r_flat),
    "reins-silhouette.svg": svg(vb, f'<path d="{sil}" fill="#000000" fill-rule="evenodd"/>'),
}
flat_bg = "#0d0d0d"  # the texture's average
def flat_icon(rx):
    clip = f'<clipPath id="{P}flat-canvas"><rect width="1254" height="1254" rx="{rx}"/></clipPath>'
    body = (f'<g clip-path="url(#{P}flat-canvas)"><rect width="1254" height="1254" fill="{flat_bg}"/>'
            f'<g transform="{place}">{r_flat}</g></g>')
    return svg("0 0 1254 1254", body, clip)
out["reins-icon-flat.svg"] = flat_icon(0)
for name, text in out.items():
    (brand / name).write_text(text)

# Scratch SVGs for the renders.
rounded = 278  # 22.2 %: the corner the app icon has had on the desktop and in the README
w = {}
w["full"] = src
w["full-rounded"] = src.replace(f'<clipPath id="{P}canvas"><rect width="1254" height="1254" rx="0"/>',
                                f'<clipPath id="{P}canvas"><rect width="1254" height="1254" rx="{rounded}"/>')
assert w["full-rounded"] != src, "the source's canvas clip changed"
w["flat"] = flat_icon(0)
w["flat-rounded"] = flat_icon(rounded)
# The background alone: the source without the R.
w["background"] = re.sub(r'<g transform="' + re.escape(place) + r'">.*?</g>\s*</g>\s*</g>(\s*</g>\s*</svg>)',
                         r"\1", src, flags=re.S)
assert w["background"] != src
# Android's foreground: the R (with its depth and grain, as on the full icon) on a 108 grid, its circle 66 across.
k = 33 / radius
fg_place = f"translate(54 54) scale({k:.5f}) translate({-cx:g} {-cy:g})"
w["foreground"] = svg("0 0 108 108", f'<g transform="{fg_place}">{r_group}</g>')
# How a themed launcher shows the monochrome layer (for the contact sheet): a tinted R on a pale circle.
w["themed"] = svg("18 18 72 72", f'<circle cx="54" cy="54" r="36" fill="#D9E2FF"/><g transform="{fg_place}">'
                  f'<path d="{sil}" fill="#1B2D53" fill-rule="evenodd"/></g>')
# The tray: the R's silhouette, `h` of a 24 grid tall, on a plate or alone. A state that needs the user adds a badge in
# the bottom-right corner (a white disc cut out of the plate, the state's sign in the plate's colour): it is what
# tells paused from Bypass (both red) and needs-you from Lockdown (both amber) at 16 to 22 px.
BADGE = {
    # waiting: a clock's hands
    "clock": '<path d="M17.6 15.2V17.9H19.8" fill="none" stroke="{c}" stroke-width="1.3" stroke-linecap="round" '
             'stroke-linejoin="round"/>',
    # paused: two bars
    "pause": '<rect x="15.75" y="15.5" width="1.3" height="4.2" rx="0.4" fill="{c}"/>'
             '<rect x="18.15" y="15.5" width="1.3" height="4.2" rx="0.4" fill="{c}"/>',
    # needs the user: an exclamation mark
    "alert": '<rect x="16.95" y="14.9" width="1.3" height="3.1" rx="0.65" fill="{c}"/>'
             '<circle cx="17.6" cy="19.75" r="0.8" fill="{c}"/>',
}
def tray(fill, plate, h, badge=None):
    s = h / (y1 - y0)
    at = "11 11" if badge else "12 12"  # with a badge, the R steps up and left to keep its leg
    g = f'<g transform="translate({at}) scale({s:.5f}) translate({-cx:g} {-cy:g})">'
    if fill == "colour":
        body = g + r_flat + "</g>"
    else:
        body = g + f'<path d="{sil}" fill="{fill}" fill-rule="evenodd"/></g>'
    rect = f'<rect x="1" y="1" width="22" height="22" rx="5.5" fill="{plate}"/>' if plate else ""
    if badge:
        body += (f'<circle cx="17.6" cy="17.6" r="5.7" fill="{plate}"/><circle cx="17.6" cy="17.6" r="4.6" fill="#FFFFFF"/>'
                 + BADGE[badge].replace("{c}", plate))
    return svg("0 0 24 24", rect + body)
w["tray-template"] = tray("#000000", None, 20)
w["tray-plate-on"] = tray("colour", "#16151B", 15)
for mode, colour in (("bypass", "#B91C1C"), ("overnight", "#5B43E8"), ("lockdown", "#A16207")):
    w[f"tray-plate-{mode}"] = tray("#FFFFFF", colour, 14)
for state, colour, badge in (("waiting", "#C2410C", "clock"), ("paused", "#B91C1C", "pause"),
                             ("pair", "#A16207", "alert")):
    w[f"tray-plate-{state}"] = tray("#FFFFFF", colour, 13, badge)
for name, text in w.items():
    (work / f"{name}.svg").write_text(text)

# The desktop's in-app mark: the flat icon, rounded (GPUI draws it as an image).
(root / "crates/reins-desktop-app/assets/icons/logos/reins.svg").write_text(flat_icon(rounded))

# Android vectors: the silhouette, placed like the foreground (themed icons) and on the 24 dp status bar grid.
def vector(size, scale, tx, ty, colour, comment):
    return f'''<?xml version="1.0" encoding="utf-8"?>
<!-- {comment}
     Generated by scripts/brand/render-logo.sh from design/brand/reins-origami-r.svg. -->
<vector xmlns:android="http://schemas.android.com/apk/res/android"
    android:width="{size}dp"
    android:height="{size}dp"
    android:viewportWidth="{size}"
    android:viewportHeight="{size}">
    <group
        android:translateX="{tx:.3f}"
        android:translateY="{ty:.3f}"
        android:scaleX="{scale:.5f}"
        android:scaleY="{scale:.5f}">
        <path
            android:fillColor="{colour}"
            android:fillType="evenOdd"
            android:pathData="{sil}" />
    </group>
</vector>
'''
res = root / "android/app/src/main/res"
(res / "drawable/ic_launcher_monochrome.xml").write_text(
    vector(108, k, 54 - k * cx, 54 - k * cy, "#FF000000",
           "The themed icon: the R's silhouette where the launcher foreground puts the R."))
n = 20 / (y1 - y0)
(res / "drawable/ic_notification.xml").write_text(
    vector(24, n, 12 - n * cx, 12 - n * cy, "#FFFFFFFF", "Status-bar icon: the R's silhouette, 20 dp tall."))
PY

# svg FILE SIZE OUT
svg() { rsvg-convert -w "$2" -h "$2" -o "$3" "$1"; }
# ImageMagick's PNG writes leave out the date chunks, so a rerun writes the same bytes.
noalpha() {
    magick "$1" -background black -alpha remove -alpha off -define png:color-type=2 \
        -define png:exclude-chunks=date,time "$1"
}

# ---- iOS -------------------------------------------------------------------------------------------------------------
xc="$root/ios/Reins/Assets.xcassets"
svg "$work/full.svg" 1024 "$xc/AppIcon.appiconset/AppIcon.png"
noalpha "$xc/AppIcon.appiconset/AppIcon.png"

imageset() { # dir: writes Contents.json for the files already in it (name@Nx.png, name-dark@Nx.png)
    python3 - "$1" <<'PY'
import json, sys
from pathlib import Path
d = Path(sys.argv[1])
images = []
for f in sorted(d.glob("*.png"), key=lambda f: ("-dark@" in f.name, f.name)):  # any appearance, then dark
    stem, scale = f.stem.rsplit("@", 1)
    entry = {"filename": f.name, "idiom": "universal", "scale": scale}
    if stem.endswith("-dark"):
        entry["appearances"] = [{"appearance": "luminosity", "value": "dark"}]
    images.append(entry)
contents = {"images": images, "info": {"author": "xcode", "version": 1}}
(d / "Contents.json").write_text(json.dumps(contents, indent=2) + "\n")
PY
}
mark="$xc/ReinsMark.imageset"
rm -rf "$mark" && mkdir -p "$mark"
for x in 1 2 3; do
    svg "$brand/reins-mark-light.svg" $((56 * x)) "$mark/reins-mark@${x}x.png"
    svg "$brand/reins-mark.svg" $((56 * x)) "$mark/reins-mark-dark@${x}x.png"
done
imageset "$mark"
icon="$xc/ReinsIcon.imageset"
rm -rf "$icon" && mkdir -p "$icon"
for x in 1 2 3; do
    svg "$work/flat.svg" $((60 * x)) "$icon/reins-icon@${x}x.png"
    noalpha "$icon/reins-icon@${x}x.png"
done
imageset "$icon"

# ---- Android ---------------------------------------------------------------------------------------------------------
res="$root/android/app/src/main/res"
for d in mdpi:108 hdpi:162 xhdpi:216 xxhdpi:324 xxxhdpi:432; do
    mkdir -p "$res/mipmap-${d%%:*}"
    svg "$work/background.svg" "${d#*:}" "$res/mipmap-${d%%:*}/ic_launcher_background.png"
    noalpha "$res/mipmap-${d%%:*}/ic_launcher_background.png"
    svg "$work/foreground.svg" "${d#*:}" "$res/mipmap-${d%%:*}/ic_launcher_foreground.png"
done
play="$root/android/play/graphics"
cp "$src" "$play/icon.svg"
svg "$play/icon.svg" 512 "$play/icon.png"
magick "$play/icon.png" -define png:color-type=6 -define png:exclude-chunks=date,time "$play/icon.png" # Play asks for 32-bit

# ---- desktop ---------------------------------------------------------------------------------------------------------
icons="$root/crates/reins-desktop-app/assets/icons"
svg "$work/full-rounded.svg" 1024 "$icons/app-1024.png"
svg "$work/full-rounded.svg" 256 "$icons/app-256.png"
svg "$work/full-rounded.svg" 824 "$work/plate.png"
magick -size 1024x1024 xc:none "$work/plate.png" -gravity center -composite \
    -define png:exclude-chunks=date,time "$icons/app-macos-1024.png"
for s in 16 24 32 48 64; do svg "$work/flat-rounded.svg" "$s" "$work/ico-$s.png"; done
for s in 128 256; do svg "$work/full-rounded.svg" "$s" "$work/ico-$s.png"; done
magick "$work"/ico-{16,24,32,48,64,128,256}.png "$root/crates/reins-desktop-app/packaging/windows/Reins.ico"
svg "$work/tray-template.svg" 36 "$icons/tray-on.png"
svg "$work/tray-plate-on.svg" 64 "$icons/tray-color-on.png"
for state in bypass overnight lockdown waiting paused pair; do
    svg "$work/tray-plate-$state.svg" 36 "$icons/tray-$state.png"
    svg "$work/tray-plate-$state.svg" 64 "$icons/tray-color-$state.png"
done

# ---- docs ------------------------------------------------------------------------------------------------------------
cp "$work/full-rounded.svg" "$root/docs/assets/logo.svg"
svg "$work/full-rounded.svg" 256 "$root/docs/assets/logo.png"

# ---- the contact sheet -----------------------------------------------------------------------------------------------
if [[ -n "$sheet" ]]; then
    sizes=(16 24 32 48 64 128 512)
    # row LABEL BG SVG-FOR-SMALL SVG-FOR-LARGE: one variant at every size on BG (small: up to 32 px).
    row() {
        local label="$1" bg="$2" small="$3" large="$4" cells=() s file
        for s in "${sizes[@]}"; do
            file="$large"
            ((s <= 32)) && file="$small"
            svg "$file" "$s" "$work/cell.png"
            magick -size "$((s + 24))x536" "xc:$bg" "$work/cell.png" -gravity center -composite "$work/c-$s.png"
            cells+=("$work/c-$s.png")
        done
        magick -size 230x536 "xc:$bg" -gravity west -fill "$([[ $bg == "#F6F6F8" ]] && echo '#27272C' || echo '#E8E8EA')" \
            -pointsize 15 -annotate +14+0 "$label" "${cells[@]}" +append "$work/row-$label-${bg#\#}.png"
        echo "$work/row-$label-${bg#\#}.png"
    }
    rows=()
    white_sil="$work/silhouette-white.svg"
    sed 's/fill="#000000"/fill="#FFFFFF"/' "$brand/reins-silhouette.svg" >"$white_sil"
    for bg in "#F6F6F8" "#111113"; do
        rows+=("$(row "app icon (flat <=64)" "$bg" "$work/flat-rounded.svg" "$work/full-rounded.svg")")
        if [[ $bg == "#F6F6F8" ]]; then
            rows+=("$(row "R mark, light" "$bg" "$brand/reins-mark-light-small.svg" "$brand/reins-mark-light.svg")")
            rows+=("$(row "silhouette" "$bg" "$brand/reins-silhouette.svg" "$brand/reins-silhouette.svg")")
        else
            rows+=("$(row "R mark, dark" "$bg" "$brand/reins-mark.svg" "$brand/reins-mark.svg")")
            rows+=("$(row "silhouette" "$bg" "$white_sil" "$white_sil")")
        fi
    done
    # cell IMAGE: one image centred on a 536 px tall cell of the sheet's grey.
    cell() {
        local out
        out="$work/x-$(basename "$1")"
        magick -size "$(($(magick identify -format %w "$1") + 40))x536" xc:'#2A2A30' "$1" -gravity center -composite \
            "$out"
        echo "$out"
    }
    # Android: the adaptive icon in the 72 dp circle mask (512 and 48 px), a themed icon (48), the notification icon
    # (24, 48, 96); then the tray icons as drawn (36 and 64 px).
    svg "$work/background.svg" 512 "$work/a-bg.png"
    svg "$work/foreground.svg" 512 "$work/a-fg.png"
    magick "$work/a-bg.png" "$work/a-fg.png" -composite -alpha off \
        \( -size 512x512 xc:black -fill white -draw "circle 256,256 256,85" -alpha off \) -compose CopyOpacity -composite \
        -trim +repage "$work/adaptive.png"
    magick "$work/adaptive.png" -resize 48x48 "$work/adaptive-48.png"
    svg "$work/themed.svg" 48 "$work/themed-48.png"
    extras=("$(cell "$work/adaptive.png")" "$(cell "$work/adaptive-48.png")" "$(cell "$work/themed-48.png")")
    for s in 24 48 96; do
        rsvg-convert -w "$s" -h "$s" -o "$work/notif-$s.png" "$white_sil"
        extras+=("$(cell "$work/notif-$s.png")")
    done
    for t in tray-on tray-color-on tray-color-bypass tray-color-overnight tray-color-lockdown tray-color-waiting \
        tray-color-paused tray-color-pair; do
        if [[ $t == tray-on ]]; then
            magick "$icons/$t.png" -background '#E8E8EA' -alpha remove -alpha off "$work/tray-on-shown.png"
            extras+=("$(cell "$work/tray-on-shown.png")")
        else
            extras+=("$(cell "$icons/$t.png")")
        fi
    done
    magick "${extras[@]}" +append "$work/extras.png"
    # The tray at the sizes it is shown (16 and 22 px), in pairs that share a colour: Bypass and paused, Lockdown and
    # needs-you; then on, Overnight and waiting.
    small=()
    for size in 16 22; do
        for t in bypass paused lockdown pair on overnight waiting; do
            svg "$work/tray-plate-$t.svg" "$size" "$work/s-$t-$size.png"
            magick -size 64x96 xc:'#2A2A30' "$work/s-$t-$size.png" -gravity center -composite "$work/sc-$t-$size.png"
            small+=("$work/sc-$t-$size.png")
        done
        magick -size 40x96 xc:'#2A2A30' "$work/gap-$size.png"
        small+=("$work/gap-$size.png")
    done
    magick "${small[@]}" +append "$work/small.png"
    magick -background '#2A2A30' "${rows[@]}" "$work/extras.png" "$work/small.png" -gravity west -append +repage "$sheet"
    echo "contact sheet: $sheet"
fi
