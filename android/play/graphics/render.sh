#!/usr/bin/env bash
# Renders the Google Play listing's graphics in this directory (android/PLAY_STORE.md, "Store listing"):
#
#   icon.png                 512 x 512 32-bit (RGBA), from icon.svg (rsvg-convert)
#   feature-graphic.png      1024 x 500 24-bit (no alpha), from feature-graphic.svg (headless Chromium, for the Geist faces it loads)
#   phone-screenshots/*.png  1215 x 2160 (9:16) 24-bit, the `play` build's screens (PlayStoreScreenshots, Robolectric)
#
#   android/play/graphics/render.sh                 # all three
#   android/play/graphics/render.sh icon feature    # only these
#
# The screenshots need what the Android unit tests need (android/README.md); REINS_PREBUILT_NATIVE=dir uses
# prebuilt native outputs (scripts/build-android-native.sh) instead of building the core.
set -euo pipefail
here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
android="$(cd "$here/../.." && pwd)"
what=("$@")
((${#what[@]})) || what=(icon feature screenshots)

# The screens shown, in order: <test method>:<file it writes>.
SCREENS=(
    activity:1-activity
    approvalSheet:2-approval
    sendSheet:5-send
    gitPush:28-git-push
    suggestionStrip:74-suggestion
    grants:9-grants
    autopilotModes:60-autopilot-assisted
    desktopPairing:27-desktop-pairing
)

for item in "${what[@]}"; do
    case "$item" in
    icon)
        rsvg-convert -w 512 -h 512 "$here/icon.svg" -o "$here/icon.png"
        magick "$here/icon.png" -define png:color-type=6 "$here/icon.png" # Play asks for 32-bit
        ;;
    feature)
        browser="$(command -v chromium || command -v chromium-browser || command -v google-chrome)"
        "$browser" --headless=new --disable-gpu --hide-scrollbars --force-device-scale-factor=1 \
            --window-size=1024,500 --screenshot="$here/feature-graphic.png" "file://$here/feature-graphic.svg" \
            2>/dev/null
        ;;
    screenshots)
        shots="$(mktemp -d)"
        trap 'rm -rf "$shots"' EXIT
        tests=()
        for screen in "${SCREENS[@]}"; do tests+=(--tests "*PlayStoreScreenshots.${screen%%:*}"); done
        native=()
        [[ -n "${REINS_PREBUILT_NATIVE:-}" ]] && native=("-Preins.prebuiltNativeDir=$REINS_PREBUILT_NATIVE")
        (cd "$android" && ./gradlew -q :app:testPlayDebugUnitTest "${tests[@]}" ${native[@]+"${native[@]}"} \
            "-Dreins.screenshots=$shots")
        rm -f "$here"/phone-screenshots/*.png
        n=1
        for screen in "${SCREENS[@]}"; do
            name="${screen#*:}"
            # Play takes 24-bit PNGs (no alpha) for screenshots.
            magick "$shots/$name-play.png" -background '#060606' -alpha remove -alpha off \
                -define png:color-type=2 "$here/phone-screenshots/$n-${name#*-}.png"
            n=$((n + 1))
        done
        ;;
    *)
        echo "unknown: $item (icon, feature, screenshots)" >&2
        exit 2
        ;;
    esac
done
file "$here"/*.png "$here"/phone-screenshots/*.png 2>/dev/null | sed "s|$here/||"
