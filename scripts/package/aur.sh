#!/usr/bin/env bash
# Writes the AUR package reins-bin of one release: PKGBUILD and .SRCINFO, from the release's GitHub assets and their
# SHA-256 in its SHA256SUMS. The package installs the release's files as they are:
#
#   x86_64, aarch64   reins       from reins-desktop-<version>-<arch>-unknown-linux-musl.tar.gz (static)
#   x86_64            reins-app   with its .desktop file and icon, from Reins-<version>-Linux-x86_64.tar.gz
#
#   scripts/package/aur.sh --version 0.2.5 --sums SHA256SUMS --out aur [--pkgrel 1]
#
# .SRCINFO is written here from the same values, so this runs without Arch Linux; it is what
# `makepkg --printsrcinfo` prints for the PKGBUILD (the release workflow and scripts/test-linux-packages.py check that).
# The release workflow pushes both to ssh://aur@aur.archlinux.org/reins-bin.git.
set -euo pipefail

die() {
    echo "aur: $*" >&2
    exit 1
}

version="" sums="" out="" pkgrel=1
while (($#)); do
    case "$1" in
    --version) version="$2" ;;
    --sums) sums="$2" ;;
    --out) out="$2" ;;
    --pkgrel) pkgrel="$2" ;;
    *) die "unknown argument $1" ;;
    esac
    shift 2
done
[[ "$version" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]] || die "--version X.Y.Z is required"
[[ -f "$sums" ]] || die "--sums must be the release's SHA256SUMS"
[[ -n "$out" ]] || die "--out is required"
[[ "$pkgrel" =~ ^[1-9][0-9]*$ ]] || die "--pkgrel must be a positive number"

repo_url="https://github.com/katulevskiy/reins"
release="$repo_url/releases/download/v$version"
pkgdesc="Phone approval for what AI agents do on this computer (command line and desktop app)"
cli_x86_64="reins-desktop-$version-x86_64-unknown-linux-musl.tar.gz"
cli_aarch64="reins-desktop-$version-aarch64-unknown-linux-musl.tar.gz"
app_x86_64="Reins-$version-Linux-x86_64.tar.gz"
sum() {
    local s
    s="$(awk -v n="$1" '$2 == n || $2 == "*" n { print $1; exit }' "$sums")"
    [[ "$s" =~ ^[0-9a-f]{64}$ ]] || die "$1 is not in $sums"
    printf '%s' "$s"
}
sum_cli_x86_64="$(sum "$cli_x86_64")"
sum_app_x86_64="$(sum "$app_x86_64")"
sum_cli_aarch64="$(sum "$cli_aarch64")"
depends_x86_64=(glibc libgcc libxcb libxkbcommon libxkbcommon-x11 wayland vulkan-icd-loader hicolor-icon-theme)
optdepends_x86_64="vulkan-driver: draws the window of the Reins app"

mkdir -p "$out"
cat >"$out/PKGBUILD" <<PKGBUILD
# Maintainer: Daniil Katulevskiy <support@reins2fa.com>
# Written for every release by scripts/package/aur.sh in $repo_url; change it there.
pkgname=reins-bin
pkgver=$version
pkgrel=$pkgrel
pkgdesc='$pkgdesc'
arch=('x86_64' 'aarch64')
url='https://reins2fa.com'
license=('Apache-2.0' 'OFL-1.1')
provides=('reins')
conflicts=('reins')
# The desktop app (x86_64) links libxcb and libxkbcommon and loads libwayland-client and libvulkan (dlopen, so namcap
# does not see those two).
depends_x86_64=($(printf "'%s' " "${depends_x86_64[@]}" | sed 's/ $//'))
optdepends_x86_64=('$optdepends_x86_64')
options=('!debug')
_release="$repo_url/releases/download/v\$pkgver"
source_x86_64=("\$_release/reins-desktop-\$pkgver-x86_64-unknown-linux-musl.tar.gz"
               "\$_release/Reins-\$pkgver-Linux-x86_64.tar.gz")
source_aarch64=("\$_release/reins-desktop-\$pkgver-aarch64-unknown-linux-musl.tar.gz")
sha256sums_x86_64=('$sum_cli_x86_64'
                   '$sum_app_x86_64')
sha256sums_aarch64=('$sum_cli_aarch64')

package() {
  local cli="reins-desktop-\$pkgver-\$CARCH-unknown-linux-musl"
  install -Dm755 "\$cli/reins" "\$pkgdir/usr/bin/reins"
  install -Dm644 "\$cli/README.md" "\$pkgdir/usr/share/doc/\$pkgname/README.md"
  install -Dm644 "\$cli/LICENSE" "\$pkgdir/usr/share/licenses/\$pkgname/LICENSE"
  install -Dm644 "\$cli/NOTICE" "\$pkgdir/usr/share/licenses/\$pkgname/NOTICE"
  if [[ \$CARCH == x86_64 ]]; then
    local app="Reins-\$pkgver-Linux-x86_64"
    install -Dm755 "\$app/reins-app" "\$pkgdir/usr/bin/reins-app"
    install -Dm644 "\$app/reins.desktop" "\$pkgdir/usr/share/applications/reins.desktop"
    install -Dm644 "\$app/reins.png" "\$pkgdir/usr/share/icons/hicolor/256x256/apps/reins.png"
    install -Dm644 "\$app/FONTS-NOTICE.txt" "\$pkgdir/usr/share/licenses/\$pkgname/FONTS-NOTICE.txt"
  fi
}
PKGBUILD

# makepkg --printsrcinfo's layout: the base's fields, then each architecture's, then the package.
{
    printf 'pkgbase = reins-bin\n'
    printf '\tpkgdesc = %s\n' "$pkgdesc"
    printf '\tpkgver = %s\n' "$version"
    printf '\tpkgrel = %s\n' "$pkgrel"
    printf '\turl = https://reins2fa.com\n'
    printf '\tarch = %s\n' x86_64 aarch64
    printf '\tlicense = %s\n' Apache-2.0 OFL-1.1
    printf '\tprovides = reins\n'
    printf '\tconflicts = reins\n'
    printf '\toptions = !debug\n'
    printf '\tsource_x86_64 = %s\n' "$release/$cli_x86_64" "$release/$app_x86_64"
    printf '\tdepends_x86_64 = %s\n' "${depends_x86_64[@]}"
    printf '\toptdepends_x86_64 = %s\n' "$optdepends_x86_64"
    printf '\tsha256sums_x86_64 = %s\n' "$sum_cli_x86_64" "$sum_app_x86_64"
    printf '\tsource_aarch64 = %s\n' "$release/$cli_aarch64"
    printf '\tsha256sums_aarch64 = %s\n' "$sum_cli_aarch64"
    printf '\npkgname = reins-bin\n'
} >"$out/.SRCINFO"
echo "Wrote $out/PKGBUILD and $out/.SRCINFO for reins-bin $version-$pkgrel"
