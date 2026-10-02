#!/usr/bin/env bash
# Builds the Reins desktop app for Linux: an AppImage (one file that runs on most distributions) and a plain tarball.
#
#   scripts/package/linux.sh            # this machine's architecture (x86_64 or aarch64)
#
# Writes to target/package/ (or $CARGO_TARGET_DIR/package/):
#   Reins-<version>-Linux-<arch>.AppImage and Reins-Linux-<arch>.AppImage (the name "latest" download links use)
#   Reins-<version>-Linux-<arch>.tar.gz and Reins-Linux-<arch>.tar.gz: reins-app, rewarden, reins.desktop, reins.png
#
# Needs GPUI's build dependencies (Debian/Ubuntu: libxkbcommon-dev libxkbcommon-x11-dev libwayland-dev libx11-dev
# libxcb1-dev libx11-xcb-dev libfontconfig1-dev libfreetype-dev libvulkan-dev pkg-config) and downloads linuxdeploy
# (https://github.com/linuxdeploy/linuxdeploy), which copies the libraries the app needs that a desktop may lack into
# the AppImage. Runs in a container too (no FUSE needed: linuxdeploy is unpacked and run).
# REINS_VERSION overrides the version; REINS_SKIP_BUILD=1 packages what is built.
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$root"
[ "$(uname -s)" = Linux ] || {
    echo "linux.sh runs on Linux" >&2
    exit 1
}
arch="${ARCH:-$(uname -m)}"
case "$arch" in
x86_64 | aarch64) ;;
*)
    echo "unsupported architecture $arch" >&2
    exit 1
    ;;
esac
triple="$arch-unknown-linux-gnu"
target_dir="${CARGO_TARGET_DIR:-$root/target}"
out="$target_dir/package"
version="${REINS_VERSION:-$(perl -ne 'if (/^version = "([^"]+)"/) { print $1; exit }' crates/rewarden-desktop/Cargo.toml)}"
assets="$root/crates/rewarden-desktop-app/assets/icons"
desktop="$root/crates/rewarden-desktop-app/packaging/linux/reins.desktop"
mkdir -p "$out"

say() { printf '==> %s\n' "$*"; }

if [ "${REINS_SKIP_BUILD:-}" != 1 ]; then
    say "building for $triple"
    cargo build --locked --profile release-app -p rewarden-desktop-app --bin reins-app --target "$triple"
    cargo build --locked --release -p rewarden-desktop --bin rewarden --target "$triple"
fi
app_bin="$target_dir/$triple/release-app/reins-app"
cli_bin="$target_dir/$triple/release/rewarden"

# The tarball.
name="Reins-$version-Linux-$arch"
stage="$out/$name"
rm -rf "$stage" && mkdir -p "$stage"
install -m 755 "$app_bin" "$stage/reins-app"
install -m 755 "$cli_bin" "$stage/rewarden"
install -m 644 "$desktop" "$stage/reins.desktop"
install -m 644 "$assets/app-256.png" "$stage/reins.png"
cp LICENSE NOTICE "$stage/"
cp ios/Shared/Fonts/FONTS-NOTICE.txt "$stage/FONTS-NOTICE.txt"
tar -czf "$out/$name.tar.gz" -C "$out" "$name"
cp -f "$out/$name.tar.gz" "$out/Reins-Linux-$arch.tar.gz"
rm -rf "$stage"

# The AppImage.
appdir="$out/Reins.AppDir"
rm -rf "$appdir"
mkdir -p "$appdir/usr/bin" "$appdir/usr/share/applications" "$appdir/usr/share/icons/hicolor/256x256/apps" \
    "$appdir/usr/share/licenses/reins"
install -m 755 "$app_bin" "$appdir/usr/bin/reins-app"
install -m 755 "$cli_bin" "$appdir/usr/bin/rewarden"
install -m 644 "$desktop" "$appdir/usr/share/applications/reins.desktop"
install -m 644 "$assets/app-256.png" "$appdir/usr/share/icons/hicolor/256x256/apps/reins.png"
cp LICENSE NOTICE ios/Shared/Fonts/FONTS-NOTICE.txt "$appdir/usr/share/licenses/reins/"

tools="$target_dir/package-tools"
mkdir -p "$tools"
linuxdeploy="$tools/linuxdeploy-$arch.AppImage"
if [ ! -x "$linuxdeploy" ]; then
    say "downloading linuxdeploy"
    curl -fsSL -o "$linuxdeploy" \
        "https://github.com/linuxdeploy/linuxdeploy/releases/download/continuous/linuxdeploy-$arch.AppImage"
    chmod 755 "$linuxdeploy"
fi
appimage="$out/$name.AppImage"
rm -f "$appimage"
say "AppImage"
(
    cd "$out"
    APPIMAGE_EXTRACT_AND_RUN=1 ARCH="$arch" LDAI_OUTPUT="$appimage" OUTPUT="$appimage" \
        "$linuxdeploy" --appdir "$appdir" \
        --executable "$appdir/usr/bin/reins-app" --executable "$appdir/usr/bin/rewarden" \
        --desktop-file "$appdir/usr/share/applications/reins.desktop" \
        --icon-file "$appdir/usr/share/icons/hicolor/256x256/apps/reins.png" \
        --output appimage
)
[ -f "$appimage" ] || {
    echo "linuxdeploy made no $appimage" >&2
    exit 1
}
chmod 755 "$appimage"
cp -f "$appimage" "$out/Reins-Linux-$arch.AppImage"
rm -rf "$appdir"

# It starts (without a display it only prints its version).
APPIMAGE_EXTRACT_AND_RUN=1 "$appimage" --version

say "Reins $version for $arch"
ls -l "$out"/Reins-*Linux-"$arch".* | awk '{printf "    %s  %s bytes\n", $NF, $5}'
