#!/usr/bin/env bash
# Builds the Reins desktop app for macOS: Reins.app (the app, with the `rewarden` command line program inside it)
# in a disk image with the drag-to-Applications window.
#
#   scripts/package/macos.sh                      # this Mac's architecture
#   ARCHS="aarch64 x86_64" scripts/package/macos.sh   # a universal app (both Rust targets installed)
#
# Writes to target/package/ (or $CARGO_TARGET_DIR/package/):
#   Reins.app
#   Reins-<version>-macOS.dmg and Reins-macOS.dmg (the same file, under the name the "latest" download links use)
#
# Signing and notarization only happen when their variables are set; without them the app is signed ad hoc (it runs
# on Apple silicon, and Gatekeeper asks once: right-click → Open, or System Settings → Privacy & Security → Open Anyway).
#   REINS_DEVELOPER_ID      "Developer ID Application: Name (TEAMID)", a signing identity in the keychain
#   REINS_NOTARY_PROFILE    a notarytool keychain profile (xcrun notarytool store-credentials), or instead
#   REINS_NOTARY_KEY, REINS_NOTARY_KEY_ID, REINS_NOTARY_ISSUER   an App Store Connect API key (.p8 path, key id, issuer)
# Other knobs: REINS_VERSION (default: crates/rewarden-desktop's version), REINS_SKIP_BUILD=1 (package what is built).
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$root"
[ "$(uname -s)" = Darwin ] || {
    echo "macos.sh runs on macOS (it needs codesign, iconutil and hdiutil)" >&2
    exit 1
}
target_dir="${CARGO_TARGET_DIR:-$root/target}"
out="$target_dir/package"
version="${REINS_VERSION:-$(perl -ne 'if (/^version = "([^"]+)"/) { print $1; exit }' crates/rewarden-desktop/Cargo.toml)}"
host_arch="$(uname -m)"
[ "$host_arch" = arm64 ] && host_arch=aarch64
read -r -a archs <<<"${ARCHS:-$host_arch}"
assets="$root/crates/rewarden-desktop-app/assets/icons"
packaging="$root/crates/rewarden-desktop-app/packaging/macos"

say() { printf '==> %s\n' "$*"; }

# 1. Build both programs for each architecture.
bins_app=() bins_cli=()
for arch in "${archs[@]}"; do
    triple="$arch-apple-darwin"
    if [ "${REINS_SKIP_BUILD:-}" != 1 ]; then
        say "building for $triple"
        cargo build --locked --profile release-app -p rewarden-desktop-app --bin reins-app --target "$triple"
        cargo build --locked --release -p rewarden-desktop --bin rewarden --target "$triple"
    fi
    bins_app+=("$target_dir/$triple/release-app/reins-app")
    bins_cli+=("$target_dir/$triple/release/rewarden")
done

# 2. The bundle.
app="$out/Reins.app"
rm -rf "$app"
mkdir -p "$app/Contents/MacOS" "$app/Contents/Resources/licenses"
if [ "${#archs[@]}" -gt 1 ]; then
    lipo -create -output "$app/Contents/MacOS/Reins" "${bins_app[@]}"
    lipo -create -output "$app/Contents/MacOS/rewarden" "${bins_cli[@]}"
else
    install -m 755 "${bins_app[0]}" "$app/Contents/MacOS/Reins"
    install -m 755 "${bins_cli[0]}" "$app/Contents/MacOS/rewarden"
fi
sed "s/__VERSION__/$version/g" "$packaging/Info.plist" >"$app/Contents/Info.plist"
plutil -lint "$app/Contents/Info.plist" >/dev/null
printf 'APPL????' >"$app/Contents/PkgInfo"
cp LICENSE NOTICE "$app/Contents/Resources/licenses/"
cp ios/Shared/Fonts/FONTS-NOTICE.txt "$app/Contents/Resources/licenses/fonts.txt"

iconset="$out/Reins.iconset"
rm -rf "$iconset" && mkdir -p "$iconset"
for size in 16 32 128 256 512; do
    sips -z "$size" "$size" "$assets/app-macos-1024.png" --out "$iconset/icon_${size}x${size}.png" >/dev/null
    sips -z $((size * 2)) $((size * 2)) "$assets/app-macos-1024.png" --out "$iconset/icon_${size}x${size}@2x.png" >/dev/null
done
iconutil -c icns "$iconset" -o "$app/Contents/Resources/Reins.icns"
rm -rf "$iconset"

# 3. Sign: the inner programs first, then the bundle. Hardened runtime and a timestamp are what notarization needs.
sign() {
    if [ -n "${REINS_DEVELOPER_ID:-}" ]; then
        codesign --force --options runtime --timestamp --sign "$REINS_DEVELOPER_ID" "$@"
    else
        codesign --force --sign - "$@"
    fi
}
if [ -n "${REINS_DEVELOPER_ID:-}" ]; then
    say "signing with $REINS_DEVELOPER_ID"
else
    say "REINS_DEVELOPER_ID is not set: signing ad hoc (not notarized; Gatekeeper asks once)"
fi
sign "$app/Contents/MacOS/rewarden"
sign "$app/Contents/MacOS/Reins"
sign "$app"
codesign --verify --strict --verbose=1 "$app"

notarize() {
    if [ -z "${REINS_DEVELOPER_ID:-}" ]; then
        return 1
    elif [ -n "${REINS_NOTARY_PROFILE:-}" ]; then
        xcrun notarytool submit "$1" --keychain-profile "$REINS_NOTARY_PROFILE" --wait
    elif [ -n "${REINS_NOTARY_KEY:-}" ] && [ -n "${REINS_NOTARY_KEY_ID:-}" ] && [ -n "${REINS_NOTARY_ISSUER:-}" ]; then
        xcrun notarytool submit "$1" --key "$REINS_NOTARY_KEY" --key-id "$REINS_NOTARY_KEY_ID" \
            --issuer "$REINS_NOTARY_ISSUER" --wait
    else
        return 1
    fi
}
notarized=false
zip="$out/Reins-notarize.zip"
ditto -c -k --keepParent "$app" "$zip"
if notarize "$zip"; then
    # A rejection can still exit 0 with some notarytool versions; stapling then fails and stops the build.
    xcrun stapler staple "$app"
    notarized=true
else
    say "not notarizing (needs REINS_DEVELOPER_ID and REINS_NOTARY_PROFILE or the REINS_NOTARY_KEY* variables)"
fi
rm -f "$zip"

# 4. The disk image: dmgbuild writes the window layout (.DS_Store) without Finder, so it works on CI; without it,
# a plain image with the Applications link.
dmg="$out/Reins-$version-macOS.dmg"
rm -f "$dmg"
python=""
if python3 -c 'import dmgbuild' 2>/dev/null; then
    python=python3
else
    venv="$out/.dmgbuild-venv"
    if [ -x "$venv/bin/python" ] && "$venv/bin/python" -c 'import dmgbuild' 2>/dev/null; then
        python="$venv/bin/python"
    elif python3 -m venv "$venv" >/dev/null 2>&1 && "$venv/bin/pip" install --quiet dmgbuild >/dev/null 2>&1; then
        python="$venv/bin/python"
    fi
fi
background="$out/dmg-background.tiff"
tiffutil -cathidpicheck "$assets/dmg-background.png" "$assets/dmg-background@2x.png" -out "$background" >/dev/null 2>&1
if [ -n "$python" ]; then
    say "disk image (dmgbuild)"
    APP="$app" DMG="$dmg" BACKGROUND="$background" "$python" - <<'PY'
import os
import dmgbuild

app = os.environ["APP"]
dmgbuild.build_dmg(
    filename=os.environ["DMG"],
    volume_name="Reins",
    settings={
        "format": "UDZO",
        "files": [app],
        "symlinks": {"Applications": "/Applications"},
        "icon": os.path.join(app, "Contents/Resources/Reins.icns"),
        "background": os.environ["BACKGROUND"],
        "show_status_bar": False,
        "show_tab_view": False,
        "show_toolbar": False,
        "show_pathbar": False,
        "show_sidebar": False,
        "default_view": "icon-view",
        # Must match the arrow in the background (scripts/package/icons.sh); 30 more points of height for the toolbar.
        "window_rect": ((200, 120), (660, 430)),
        "icon_size": 104,
        "text_size": 12,
        # The hidden files out of sight, for Finders that show hidden files.
        "icon_locations": {
            "Reins.app": (165, 170),
            "Applications": (495, 170),
            ".background.tiff": (900, 900),
            ".VolumeIcon.icns": (900, 900),
        },
    },
)
PY
else
    say "disk image (hdiutil; install dmgbuild for the drag-to-Applications window)"
    stage="$out/dmg-stage"
    rm -rf "$stage" && mkdir -p "$stage/.background"
    cp -R "$app" "$stage/"
    ln -s /Applications "$stage/Applications"
    cp "$background" "$stage/.background/background.tiff"
    hdiutil create -quiet -volname Reins -srcfolder "$stage" -fs HFS+ -format UDZO -ov "$dmg"
    rm -rf "$stage"
fi
rm -f "$background"
if [ -n "${REINS_DEVELOPER_ID:-}" ]; then
    codesign --force --timestamp --sign "$REINS_DEVELOPER_ID" "$dmg"
fi
if $notarized && notarize "$dmg"; then
    xcrun stapler staple "$dmg"
fi
hdiutil verify -quiet "$dmg"
cp -f "$dmg" "$out/Reins-macOS.dmg"

say "Reins $version for ${archs[*]}"
du -sh "$app" | sed 's/^/    /'
ls -l "$dmg" "$out/Reins-macOS.dmg" | awk '{printf "    %s  %s bytes\n", $NF, $5}'
