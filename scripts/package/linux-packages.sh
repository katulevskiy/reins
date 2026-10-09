#!/usr/bin/env bash
# Builds the Linux distribution packages of one release with nfpm (https://nfpm.goreleaser.com), from the release's
# assets (the same files the archives and the AppImage carry; nothing is compiled here):
#
#   reins_<version>_amd64.deb      reins-<version>-1.x86_64.rpm     the command-line program (static musl)
#   reins_<version>_arm64.deb      reins-<version>-1.aarch64.rpm
#   reins-app_<version>_amd64.deb  reins-app-<version>-1.x86_64.rpm  the desktop app, x86_64 only (needs reins)
#
# from reins-desktop-<version>-{x86_64,aarch64}-unknown-linux-musl.tar.gz and Reins-<version>-Linux-x86_64.tar.gz in
# --dist. The package definitions are scripts/package/linux-packages/ (reins.yaml, reins-app.yaml). The .rpm files
# are unsigned here: scripts/package/linux-repos.sh signs them and builds the APT and RPM repositories.
#
#   scripts/package/linux-packages.sh --version 0.2.5 --build 0.2.5-202610090300-fad997a6 --time 1791515000 \
#       --dist dist [--out dist]
#
# Needs nfpm (NFPM=/path/to/nfpm, else on the PATH), strip (binutils), tar and gzip.
set -euo pipefail

die() {
    echo "linux-packages: $*" >&2
    exit 1
}

version="" build="" build_time="" dist="" out=""
while (($#)); do
    case "$1" in
    --version) version="$2" ;;
    --build) build="$2" ;;
    --time) build_time="$2" ;;
    --dist) dist="$2" ;;
    --out) out="$2" ;;
    *) die "unknown argument $1" ;;
    esac
    shift 2
done
[[ "$version" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]] || die "--version X.Y.Z is required"
[[ "$build" == "$version"-* && "$build" =~ ^[0-9A-Za-z._-]+$ ]] || die "--build must start with $version-"
[[ "$build_time" =~ ^[0-9]+$ ]] || die "--time (unix seconds) is required"
[[ -d "$dist" ]] || die "--dist must be the directory with the release assets"
out="${out:-$dist}"
mkdir -p "$out"
dist="$(cd "$dist" && pwd)"
out="$(cd "$out" && pwd)"
root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
spec="$root/scripts/package/linux-packages"
nfpm="${NFPM:-nfpm}"
command -v "$nfpm" >/dev/null || die "nfpm is needed (NFPM=/path/to/nfpm)"

stage="$(mktemp -d)"
trap 'rm -rf "$stage"' EXIT

# Debian wants a changelog in every package (gzip -9n, no timestamp); the release notes are on GitHub.
changelog() { # <package> <dest>
    {
        printf '%s (%s) stable; urgency=medium\n\n' "$1" "$version"
        printf '  * Reins %s (%s), release notes:\n    https://github.com/katulevskiy/reins/releases/tag/v%s\n\n' \
            "$version" "$build" "$version"
        printf ' -- Daniil Katulevskiy <support@reins2fa.com>  %s\n' "$(LC_ALL=C date -u -R -d "@$build_time")"
    } | gzip -9n >"$2"
}

# strip_binary <arch> <file>: symbols out (the release profiles keep the symbol table; Debian wants packaged programs
# stripped). llvm-strip handles both architectures; otherwise binutils' strip for the build machine's own, and
# aarch64-linux-gnu-strip (binutils-aarch64-linux-gnu) for arm64 on an x86_64 machine.
strip_binary() {
    local tool
    tool="$(command -v llvm-strip || compgen -c llvm-strip- | sort -V | tail -1 || true)"
    if [[ -z "$tool" ]]; then
        if [[ "$1" == "$(uname -m)" ]]; then tool=strip; else tool="$1-linux-gnu-strip"; fi
    fi
    command -v "$tool" >/dev/null || die "no strip for $1 (install llvm or binutils-$1-linux-gnu)"
    "$tool" --strip-unneeded --remove-section=.comment --remove-section=.note "$2"
}

# docs <package> <dir>: the files under /usr/share/doc/<package> (and the license, for the .rpm).
docs() {
    mkdir -p "$2/doc"
    cp "$root/LICENSE" "$root/NOTICE" "$2/doc/"
    cp "$spec/copyright-$1" "$2/doc/copyright"
    changelog "$1" "$2/doc/changelog.gz"
}

# package <config> <nfpm arch> <stage> <deb name> <rpm name>
package() {
    local config="$1" arch="$2" files="$3" deb="$4" rpm="$5"
    for packager in deb rpm; do
        local target="$out/$deb" release=""
        [[ "$packager" == rpm ]] && target="$out/$rpm" release=1
        rm -f "$target"
        # The ${NAME}s of the configuration, filled in here (nfpm expands only some fields itself). The .deb has no
        # Debian revision (reins_<version>_<arch>.deb), the .rpm the release 1 (reins-<version>-1.<arch>.rpm).
        VERSION="$version" PKG_ARCH="$arch" PKG_RELEASE="$release" STAGE="$files" SPEC="$spec" \
            perl -pe 's/\$\{([A-Z_]+)\}/exists $ENV{$1} ? $ENV{$1} : die "unknown \${$1}\n"/ge' "$config" \
            >"$stage/nfpm.yaml" || die "cannot fill in $config"
        # Files get the release's time.
        SOURCE_DATE_EPOCH="$build_time" "$nfpm" package --config "$stage/nfpm.yaml" --packager "$packager" \
            --target "$target" >/dev/null || die "nfpm could not build $(basename "$target")"
        [[ -s "$target" ]] || die "nfpm made no $target"
        echo "  $(basename "$target")"
    done
}

echo "Linux packages of Reins $version ($build):"

# ── reins: the command-line program, x86_64 and aarch64 ───────────────────────────────────────────────────────────────
for t in x86_64:amd64 aarch64:arm64; do
    IFS=: read -r arch deb_arch <<<"$t"
    case "$arch" in x86_64) rpm_arch=x86_64 ;; aarch64) rpm_arch=aarch64 ;; esac
    dir="reins-desktop-$version-$arch-unknown-linux-musl"
    [[ -f "$dist/$dir.tar.gz" ]] || die "$dir.tar.gz is missing"
    files="$stage/reins-$arch"
    mkdir -p "$files"
    tar -xzf "$dist/$dir.tar.gz" -C "$stage" "$dir/reins" "$dir/README.md" || die "$dir.tar.gz has no $dir/reins"
    mv "$stage/$dir/reins" "$files/reins"
    grep -qaF "$version ($build)" "$files/reins" || die "$dir.tar.gz: reins does not carry the build id $build"
    strip_binary "$arch" "$files/reins"
    docs reins "$files"
    mv "$stage/$dir/README.md" "$files/doc/README.md"
    package "$spec/reins.yaml" "$deb_arch" "$files" "reins_${version}_$deb_arch.deb" "reins-$version-1.$rpm_arch.rpm"
done

# ── reins-app: the desktop app, x86_64 (the only Linux build of the app) ──────────────────────────────────────────────
dir="Reins-$version-Linux-x86_64"
[[ -f "$dist/$dir.tar.gz" ]] || die "$dir.tar.gz is missing"
tar -xzf "$dist/$dir.tar.gz" -C "$stage" "$dir/reins-app" "$dir/reins.desktop" "$dir/reins.png" "$dir/FONTS-NOTICE.txt" ||
    die "$dir.tar.gz lacks the app's files"
files="$stage/reins-app"
mkdir -p "$files"
mv "$stage/$dir/reins-app" "$stage/$dir/reins.desktop" "$stage/$dir/reins.png" "$files/"
grep -qaF "$version ($build)" "$files/reins-app" || die "$dir.tar.gz: reins-app does not carry the build id $build"
# Symbols out, as linuxdeploy does for the AppImage (the release-app profile keeps them).
strip_binary x86_64 "$files/reins-app"
docs reins-app "$files"
mv "$stage/$dir/FONTS-NOTICE.txt" "$files/doc/"
package "$spec/reins-app.yaml" amd64 "$files" "reins-app_${version}_amd64.deb" "reins-app-$version-1.x86_64.rpm"
