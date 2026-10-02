#!/usr/bin/env bash
# Publishes the Rewarden desktop app to the site's release feed: builds static Linux binaries, signs the release
# manifest with the release key, uploads everything, then switches "latest" to it. Installs (install.sh) and
# `rewarden update` pick it up at once. Run from anywhere in the repository:
#
#   scripts/release-desktop.sh                       # Linux only, from the current commit; refuses uncommitted changes
#   scripts/release-desktop.sh --dirty               # publish anyway (the build id says so)
#   scripts/release-desktop.sh --macos-from v0.2.0   # the GitHub release v0.2.0, Linux and macOS
#   scripts/release-desktop.sh --macos-from latest   # the latest GitHub release, Linux and macOS
#
# macOS binaries cannot be built here, so --macos-from takes them from a GitHub release (made by
# .github/workflows/release.yml on a macOS runner): it downloads the release's SHA256SUMS and the two
# reins-desktop-X.Y.Z-{aarch64,x86_64}-apple-darwin.tar.gz archives, checks each archive against SHA256SUMS, checks the
# binary inside is a Mach-O for the right processor that carries the release's build id, and publishes it as
# macos-aarch64 / macos-x86_64 next to the Linux binaries: same manifest, signed with the same key, same
# latest-<platform>.txt pointers. The Linux binaries are built here as always, but from the release's tag (in a
# temporary git worktree, with the version set as the workflow sets it), not from the current commit.
#
# Versions with --macos-from: the whole feed release takes the GitHub release's identity, computed exactly as the
# release workflow computes it from the tagged commit:
#   version     X.Y.Z from the tag vX.Y.Z
#   build id    X.Y.Z-<commit time, UTC, YYYYMMDDhhmm>-<first 8 hex digits of the commit>
#   build time  the commit time (Unix seconds)
# The macOS binaries already carry that build id and time (the workflow builds with them), and the Linux ones are
# built with them here, so every binary in the feed reports the same `rewarden --version` as the GitHub release and
# carries the same build time as the signed manifest. `rewarden update` offers a release only when the manifest's build
# time is later than the running binary's, so a fresh install is up to date and the next release is offered once
# published. Without --macos-from the build time is the time of publishing and the version comes from
# crates/rewarden-desktop/Cargo.toml, as before; the manifest then names no macOS build (Macs keep the
# latest-macos-*.txt of the last --macos-from release, and `rewarden update` on a Mac says there is no build for it).
# Either way the script refuses to publish a release older than the one the feed already serves, since installed apps
# would never move to it.
#
# Settings, from the environment or scripts/release.env (see scripts/release.env.example):
#   REWARDEN_SITE           the site that serves the releases and install.sh (required)
#   REWARDEN_RELEASE_SSH    SSH destination to upload to (required)
#   REWARDEN_RELEASE_DIR    directory on it that the site serves as /releases (required)
#   REWARDEN_RELEASE_KEY    the release signing key (required)
#   REWARDEN_RELEASE_URL    where the releases are served ($REWARDEN_SITE/releases)
#   REWARDEN_RELEASE_KEEP   builds kept on the server per platform (10)
#   REWARDEN_GITHUB_REPO    the GitHub repository --macos-from downloads from (katulevskiy/reins)
set -euo pipefail

source "$(dirname "${BASH_SOURCE[0]}")/release-env.sh"
require_settings REWARDEN_SITE REWARDEN_RELEASE_SSH REWARDEN_RELEASE_DIR REWARDEN_RELEASE_KEY

SSH_TARGET="$REWARDEN_RELEASE_SSH"
REMOTE_DIR="$REWARDEN_RELEASE_DIR"
SITE="${REWARDEN_SITE%/}"
URL="${REWARDEN_RELEASE_URL:-$SITE/releases}"
KEY="$REWARDEN_RELEASE_KEY"
KEEP="${REWARDEN_RELEASE_KEEP:-10}"
GITHUB_REPO="${REWARDEN_GITHUB_REPO:-katulevskiy/reins}"
TARGETS=("linux-x86_64:x86_64-unknown-linux-musl" "linux-aarch64:aarch64-unknown-linux-musl")
# platform:target triple:the processor as `file` names it
MACOS_TARGETS=("macos-aarch64:aarch64-apple-darwin:arm64" "macos-x86_64:x86_64-apple-darwin:x86_64")
ALL_PLATFORMS="linux-x86_64 linux-aarch64 macos-x86_64 macos-aarch64"

usage() {
    echo "usage: $0 [--dirty | --macos-from vX.Y.Z|latest]" >&2
    exit 2
}
die() {
    echo "$*" >&2
    exit 1
}

dirty_ok=false
macos_from=""
while (($#)); do
    case "$1" in
    --dirty) dirty_ok=true ;;
    --macos-from)
        [[ -n "${2:-}" ]] || usage
        macos_from="$2"
        shift
        ;;
    --macos-from=*) macos_from="${1#*=}" ;;
    *) usage ;;
    esac
    shift
done
if [[ -n "$macos_from" ]] && $dirty_ok; then
    die "--dirty does not apply to --macos-from: a GitHub release is built from its tag"
fi

repo="$(git rev-parse --show-toplevel)"
cd "$repo"
[[ -f "$KEY" ]] || die "release key $KEY not found (make one with: cargo run -p rewarden-desktop --bin rewarden-release -- keygen $KEY)"
export CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-$repo/target}"

stage="$(mktemp -d)"
cleanup() {
    if [[ -d "$stage/src" ]]; then git -C "$repo" worktree remove --force "$stage/src" || true; fi
    rm -rf "$stage"
}
trap cleanup EXIT
mkdir -p "$stage/files"

# What to publish: the tree the Linux binaries are built from, and the release's version, build id and build time.
src="$repo"
if [[ -n "$macos_from" ]]; then
    for tool in curl file tar sha256sum; do
        command -v "$tool" >/dev/null || die "--macos-from needs $tool"
    done
    tag="$macos_from"
    if [[ "$tag" == latest ]]; then
        tag="$(curl -fsSL "https://api.github.com/repos/$GITHUB_REPO/releases/latest" |
            sed -n 's/^ *"tag_name": *"\([^"]*\)".*/\1/p' | head -1)"
        [[ -n "$tag" ]] || die "could not find the latest GitHub release of $GITHUB_REPO"
    fi
    [[ "$tag" =~ ^v(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)$ ]] || die "$tag is not a release tag (vX.Y.Z)"
    version="${tag#v}"
    # The tag as GitHub has it (git refuses to move a local tag of the same name that differs).
    git fetch -q "https://github.com/$GITHUB_REPO.git" "refs/tags/$tag:refs/tags/$tag" ||
        die "could not fetch $tag from $GITHUB_REPO (or the local $tag differs from it)"
    commit="$(git rev-parse "$tag^{commit}")"
    # As .github/workflows/release.yml computes them.
    build_time="$(git log -1 --format=%ct "$commit")"
    build="$version-$(TZ=UTC git log -1 --date=format-local:%Y%m%d%H%M --format=%cd "$commit")-${commit:0:8}"
    echo "Release $build: GitHub release $tag ($commit)"
    git worktree add -q --detach "$stage/src" "$commit"
    src="$stage/src"
    [[ -x "$src/scripts/set-desktop-version.sh" ]] || die "$tag has no scripts/set-desktop-version.sh"
    "$src/scripts/set-desktop-version.sh" "$version" >/dev/null
else
    sha="$(git rev-parse --short=8 HEAD)"
    if [[ -n "$(git status --porcelain --untracked-files=no)" ]]; then
        $dirty_ok || die "uncommitted changes; commit them or pass --dirty"
        sha="$sha.dirty"
    fi
    version="$(sed -n 's/^version *= *"\(.*\)"/\1/p' crates/rewarden-desktop/Cargo.toml | head -1)"
    build_time="$(date -u +%s)"
    build="$version-$(date -u -d "@$build_time" +%Y%m%d%H%M)-$sha"
    echo "Release $build"
fi
export REWARDEN_BUILD="$build" REWARDEN_BUILD_TIME="$build_time"

# Never older than what the feed serves: installed apps only move to a later build time.
current="$(curl -fsS "$URL/latest.json" 2>/dev/null || true)"
current_time="$(grep -o 'build_time\\":[0-9]*' <<<"$current" | grep -o '[0-9]*$' || true)"
current_build="$(grep -o 'build\\":\\"[^\\]*' <<<"$current" | sed 's/.*"//' || true)"
if [[ -n "$current_time" ]]; then
    if ((build_time < current_time)) || { ((build_time == current_time)) && [[ "$build" != "$current_build" ]]; }; then
        die "the feed already serves $current_build, which is not older (build time $current_time, this one $build_time); refusing"
    fi
fi

# The rustup toolchain the source pins (the system cargo has no musl libraries).
channel="$(sed -n 's/^channel *= *"\(.*\)"/\1/p' "$src/rust-toolchain.toml")"
toolchain_bin="$(rustup toolchain list -v | awk -v c="$channel" 'index($1, c) == 1 { print $NF; exit }')/bin"
[[ -x "$toolchain_bin/cargo" ]] || die "rustup toolchain $channel not installed (rustup toolchain install $channel)"
export PATH="$toolchain_bin:$PATH"
for t in "${TARGETS[@]}"; do rustup target add --toolchain "$channel" "${t#*:}" >/dev/null; done

# The key must be the one the apps pin, or every update would be refused.
pinned="$(sed -n 's/^pub const RELEASE_KEY: &str = "\([0-9a-f]*\)";/\1/p' crates/rewarden-desktop/src/update.rs)"
public="$(cargo run --release --locked -q -p rewarden-desktop --bin rewarden-release -- public "$KEY")"
[[ -n "$pinned" && "$public" == "$pinned" ]] || die "$KEY is not the release key the apps pin (update::RELEASE_KEY)"

assets=()
for t in "${TARGETS[@]}"; do
    platform="${t%%:*}" triple="${t#*:}"
    echo "Building $platform ($triple)..."
    (cd "$src" && cargo zigbuild --release --locked -q -p rewarden-desktop --target "$triple" --bin rewarden)
    file="rewarden-$build-$platform"
    cp "$CARGO_TARGET_DIR/$triple/release/rewarden" "$stage/files/$file"
    assets+=("$platform=$stage/files/$file")
done
host_bin="$stage/files/rewarden-$build-linux-x86_64"
"$host_bin" --version | grep -qF "$version ($build)" || die "the built binary does not report $version ($build)"

if [[ -n "$macos_from" ]]; then
    gh="https://github.com/$GITHUB_REPO/releases/download/$tag"
    mkdir -p "$stage/gh"
    curl -fsSL "$gh/SHA256SUMS" -o "$stage/gh/SHA256SUMS" || die "could not download SHA256SUMS of $tag"
    for t in "${MACOS_TARGETS[@]}"; do
        IFS=: read -r platform triple cpu <<<"$t"
        archive="reins-desktop-$version-$triple.tar.gz"
        echo "Fetching $platform ($archive from $tag)..."
        want="$(awk -v f="$archive" '$2 == f || $2 == "*" f { print $1 }' "$stage/gh/SHA256SUMS")"
        [[ "$want" =~ ^[0-9a-f]{64}$ ]] || die "SHA256SUMS of $tag does not list $archive"
        curl -fsSL "$gh/$archive" -o "$stage/gh/$archive" || die "could not download $archive"
        got="$(sha256sum "$stage/gh/$archive" | cut -d' ' -f1)"
        [[ "$got" == "$want" ]] || die "$archive does not match SHA256SUMS of $tag"
        tar -xzf "$stage/gh/$archive" -C "$stage/gh" "reins-desktop-$version-$triple/rewarden" ||
            die "$archive has no reins-desktop-$version-$triple/rewarden"
        bin="$stage/gh/reins-desktop-$version-$triple/rewarden"
        kind="$(file -b "$bin")"
        [[ "$kind" == "Mach-O 64-bit $cpu executable"* ]] || die "$archive: rewarden is not a macOS $cpu program ($kind)"
        grep -qaF "$version ($build)" "$bin" || die "$archive: rewarden does not carry the build id $build"
        file="rewarden-$build-$platform"
        cp "$bin" "$stage/files/$file"
        assets+=("$platform=$stage/files/$file")
    done
fi

cargo run --release --locked -q -p rewarden-desktop --bin rewarden-release -- manifest \
    --key "$KEY" --version "$version" --build "$build" --time "$build_time" --out "$stage" "${assets[@]}"
# The installer it serves (this checkout's) points at this site.
sed "s|^DEFAULT_SITE=.*|DEFAULT_SITE=\"$SITE\"|" scripts/install.sh >"$stage/install.sh"
grep -qxF "DEFAULT_SITE=\"$SITE\"" "$stage/install.sh" || die "could not set DEFAULT_SITE in install.sh"

echo "Uploading to $SSH_TARGET:$REMOTE_DIR..."
ssh "$SSH_TARGET" "mkdir -p '$REMOTE_DIR/files' '$REMOTE_DIR/.incoming'"
scp -q "$stage"/files/* "$SSH_TARGET:$REMOTE_DIR/files/"
scp -q "$stage"/latest.json "$stage"/latest-*.txt "$stage"/install.sh "$SSH_TARGET:$REMOTE_DIR/.incoming/"
# Binaries are in place first; each pointer then switches with an atomic rename. Old builds beyond $KEEP are removed.
ssh "$SSH_TARGET" "set -e; cd '$REMOTE_DIR'; chmod 644 files/* .incoming/*;
    for f in .incoming/*; do mv -f \"\$f\" .; done;
    for p in $ALL_PLATFORMS; do ls -1t files/ | grep -- \"-\$p\$\" | tail -n +$((KEEP + 1)) | sed 's|^|files/|' | xargs -r rm -f; done"

for spec in "${assets[@]}"; do
    platform="${spec%%=*}"
    published="$(curl -fsS "$URL/latest-$platform.txt")"
    [[ "$published" == *"$build"* ]] || die "the server does not show the new release for $platform: $published"
done
echo "Published $build for ${assets[*]%%=*}"
if [[ -z "$macos_from" ]] && curl -fsSo /dev/null "$URL/latest-macos-aarch64.txt" 2>/dev/null; then
    echo "  macOS is not part of this release: Macs keep the last one published with --macos-from"
fi
echo "  install: curl -fsSL $SITE/install.sh | sh"
echo "  update:  rewarden update"
