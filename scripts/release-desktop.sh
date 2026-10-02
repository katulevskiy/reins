#!/usr/bin/env bash
# Publishes the Rewarden desktop app from the current commit: builds static Linux binaries, signs the release manifest
# with the release key, uploads everything, then switches "latest" to it. Installs (install.sh) and `rewarden update`
# pick it up at once. Run from anywhere in the repository:
#
#   scripts/release-desktop.sh            # refuses uncommitted changes
#   scripts/release-desktop.sh --dirty    # publish anyway (the build id says so)
#
# Settings, from the environment or scripts/release.env (see scripts/release.env.example):
#   REWARDEN_SITE           the site that serves the releases and install.sh (required)
#   REWARDEN_RELEASE_SSH    SSH destination to upload to (required)
#   REWARDEN_RELEASE_DIR    directory on it that the site serves as /releases (required)
#   REWARDEN_RELEASE_KEY    the release signing key (required)
#   REWARDEN_RELEASE_URL    where the releases are served ($REWARDEN_SITE/releases)
#   REWARDEN_RELEASE_KEEP   builds kept on the server per platform (10)
set -euo pipefail

source "$(dirname "${BASH_SOURCE[0]}")/release-env.sh"
require_settings REWARDEN_SITE REWARDEN_RELEASE_SSH REWARDEN_RELEASE_DIR REWARDEN_RELEASE_KEY

SSH_TARGET="$REWARDEN_RELEASE_SSH"
REMOTE_DIR="$REWARDEN_RELEASE_DIR"
SITE="${REWARDEN_SITE%/}"
URL="${REWARDEN_RELEASE_URL:-$SITE/releases}"
KEY="$REWARDEN_RELEASE_KEY"
KEEP="${REWARDEN_RELEASE_KEEP:-10}"
TARGETS=("linux-x86_64:x86_64-unknown-linux-musl" "linux-aarch64:aarch64-unknown-linux-musl")

dirty_ok=false
[[ "${1:-}" == "--dirty" ]] && dirty_ok=true

cd "$(git rev-parse --show-toplevel)"
[[ -f "$KEY" ]] || {
    echo "release key $KEY not found (make one with: cargo run -p rewarden-desktop --bin rewarden-release -- keygen $KEY)" >&2
    exit 1
}

# The rustup toolchain the repository pins (the system cargo has no musl libraries).
channel="$(sed -n 's/^channel *= *"\(.*\)"/\1/p' rust-toolchain.toml)"
toolchain_bin="$(rustup toolchain list -v | awk -v c="$channel" 'index($1, c) == 1 { print $NF; exit }')/bin"
[[ -x "$toolchain_bin/cargo" ]] || {
    echo "rustup toolchain $channel not installed (rustup toolchain install $channel)" >&2
    exit 1
}
export PATH="$toolchain_bin:$PATH"
for t in "${TARGETS[@]}"; do rustup target add --toolchain "$channel" "${t#*:}" >/dev/null; done

sha="$(git rev-parse --short=8 HEAD)"
if [[ -n "$(git status --porcelain --untracked-files=no)" ]]; then
    $dirty_ok || {
        echo "uncommitted changes; commit them or pass --dirty" >&2
        exit 1
    }
    sha="$sha.dirty"
fi
version="$(sed -n 's/^version *= *"\(.*\)"/\1/p' crates/rewarden-desktop/Cargo.toml | head -1)"
build_time="$(date -u +%s)"
build="$version-$(date -u -d "@$build_time" +%Y%m%d%H%M)-$sha"
export REWARDEN_BUILD="$build" REWARDEN_BUILD_TIME="$build_time"
echo "Release $build"

stage="$(mktemp -d)"
trap 'rm -rf "$stage"' EXIT
mkdir -p "$stage/files"
assets=()
for t in "${TARGETS[@]}"; do
    platform="${t%%:*}" triple="${t#*:}"
    echo "Building $platform ($triple)..."
    cargo zigbuild --release --locked -q -p rewarden-desktop --target "$triple" --bin rewarden
    file="rewarden-$build-$platform"
    cp "target/$triple/release/rewarden" "$stage/files/$file"
    assets+=("$platform=$stage/files/$file")
done
host_bin="$stage/files/rewarden-$build-linux-x86_64"
"$host_bin" --version | grep -qF "$build" || {
    echo "the built binary does not report $build" >&2
    exit 1
}

cargo run --release --locked -q -p rewarden-desktop --bin rewarden-release -- manifest \
    --key "$KEY" --version "$version" --build "$build" --time "$build_time" --out "$stage" "${assets[@]}"
# The installer it serves points at this site.
sed "s|^DEFAULT_SITE=.*|DEFAULT_SITE=\"$SITE\"|" scripts/install.sh >"$stage/install.sh"
grep -qxF "DEFAULT_SITE=\"$SITE\"" "$stage/install.sh" || {
    echo "could not set DEFAULT_SITE in install.sh" >&2
    exit 1
}

echo "Uploading to $SSH_TARGET:$REMOTE_DIR..."
ssh "$SSH_TARGET" "mkdir -p '$REMOTE_DIR/files' '$REMOTE_DIR/.incoming'"
scp -q "$stage"/files/* "$SSH_TARGET:$REMOTE_DIR/files/"
scp -q "$stage"/latest.json "$stage"/latest-*.txt "$stage"/install.sh "$SSH_TARGET:$REMOTE_DIR/.incoming/"
# Binaries are in place first; each pointer then switches with an atomic rename. Old builds beyond $KEEP are removed.
ssh "$SSH_TARGET" "set -e; cd '$REMOTE_DIR'; chmod 644 files/* .incoming/*;
    for f in .incoming/*; do mv -f \"\$f\" .; done;
    for p in linux-x86_64 linux-aarch64; do ls -1t files/ | grep -- \"-\$p\$\" | tail -n +$((KEEP + 1)) | sed 's|^|files/|' | xargs -r rm -f; done"

published="$(curl -fsS "$URL/latest-linux-x86_64.txt")"
[[ "$published" == *"$build"* ]] || {
    echo "the server does not show the new release: $published" >&2
    exit 1
}
echo "Published $build"
echo "  install: curl -fsSL $SITE/install.sh | sh"
echo "  update:  rewarden update"
