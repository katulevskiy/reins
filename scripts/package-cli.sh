#!/usr/bin/env bash
# Package an already-built CLI, including the binary also bundled in a desktop installer.
set -euo pipefail
cd "$(dirname "${BASH_SOURCE[0]}")/.."
target="${1:?target triple}"
version="${2:?release version}"
profile="${3:-release}"
[[ "$target" =~ ^[a-zA-Z0-9_-]+$ && "$version" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ && "$profile" =~ ^[a-zA-Z0-9_-]+$ ]]
name="reins-desktop-$version-$target"
stage="$(mktemp -d "${RUNNER_TEMP:-${TMPDIR:-/tmp}}/reins-cli.XXXXXX")"
trap 'rm -rf -- "$stage"' EXIT
mkdir -p "$stage/$name" "${REINS_DIST_DIR:-dist}"
dist="$(cd "${REINS_DIST_DIR:-dist}" && pwd)"
binary="${CARGO_TARGET_DIR:-target}/$target/$profile/reins"
[[ "$target" == *-windows-* ]] && binary="$binary.exe"
cp "$binary" LICENSE LICENSE-AGPL LICENSING.md NOTICE README.md "$stage/$name/"
if [[ "$target" == *-windows-* ]]; then
    archive="$name.zip"
    destination="$dist/$archive"
    (cd "$stage" && 7z a -tzip -bso0 -bsp0 "$destination" "$name")
else
    archive="$name.tar.gz"
    tar -C "$stage" -czf "$dist/$archive" "$name"
fi
cp "$dist/$archive" "$dist/reins-desktop-$target.${archive#"$name".}"
