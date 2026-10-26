#!/bin/bash
# The prebuilt Rust core for iOS (the static library, its C header and module map, and the Swift bindings), one
# build per content hash of the core's sources, published as assets of the rolling `ios-core-cache` prerelease.
#
#   ios/scripts/core-cache.sh hash                 the content hash (16 hex digits)
#   ios/scripts/core-cache.sh asset <platform>     the asset's file name for this tree
#   ios/scripts/core-cache.sh fetch <platform>     download it into target/ios-core/<platform>; exit 1 when none
#   ios/scripts/core-cache.sh ensure <platform>    the stamp matches, else fetch, else build-core.sh (stamped)
#   ios/scripts/core-cache.sh install <platform> <file> unpack an archive (an artifact) into target/ios-core
#   ios/scripts/core-cache.sh pack <platform> <file>   tar the built core (for publish, or an artifact)
#   ios/scripts/core-cache.sh publish <platform> [file] upload it (needs gh with contents: write)
#
# <platform> is iphonesimulator or iphoneos. Debug builds only (build-core.sh's release-low profile): Release
# (TestFlight) always builds its own core. The hash covers what the core build reads: the three crates, the
# workspace Cargo.toml (profiles) and Cargo.lock, rust-toolchain.toml, .cargo/, uniffi.toml and build-core.sh.
# It hashes the working tree (git hash-object), so a local edit to the core changes it before it is committed.
#
# Why a release asset and not an Actions cache: caches are scoped to the branch that wrote them (plus main), evicted
# after 7 days unused or when the repository passes 10 GB, and have no download API for the Mac or this PC. A
# release asset is public, permanent until pruned, downloadable with curl from anywhere, and keyed by its name.
#
# Environment: GH_REPO (default katulevskiy/reins); REINS_CORE_PUBLISH=1 makes `ensure` publish what it built;
# REINS_CORE_KEEP (default 12) is how many hashes `publish` keeps per platform.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
REPO="${GH_REPO:-katulevskiy/reins}"
TAG=ios-core-cache
FORMAT=v1
PROFILE=release-low
INPUTS=(
  crates/reins-core crates/reins-proto crates/reins-policy
  Cargo.toml Cargo.lock rust-toolchain.toml .cargo ios/scripts/build-core.sh
)

die() { echo "core-cache: $*" >&2; exit 1; }

platform_ok() {
  case "${1:-}" in
    iphonesimulator|iphoneos) ;;
    *) die "platform must be iphonesimulator or iphoneos (got '${1:-}')" ;;
  esac
}

sha256() {
  if command -v sha256sum >/dev/null 2>&1; then sha256sum | cut -d' ' -f1; else shasum -a 256 | cut -d' ' -f1; fi
}

content_hash() {
  cd "$ROOT"
  local files
  # Tracked and untracked-but-not-ignored files, as they are on disk (deleted tracked files drop out).
  files="$(git ls-files -z --cached --others --exclude-standard -- "${INPUTS[@]}" |
    tr '\0' '\n' | LC_ALL=C sort -u | while IFS= read -r f; do [[ -f "$f" ]] && printf '%s\n' "$f"; done)"
  [[ -n "$files" ]] || die "no core sources found under $ROOT"
  {
    echo "$FORMAT $PROFILE"
    paste -d' ' <(printf '%s\n' "$files") <(printf '%s\n' "$files" | git hash-object --stdin-paths)
  } | sha256 | cut -c1-16
}

asset_name() { echo "core-$(content_hash)-$1.tar.gz"; }

out_dir() { echo "$ROOT/target/ios-core/$1"; }

stamp_ok() {
  local dir hash
  dir="$(out_dir "$1")"
  hash="$(content_hash)"
  [[ -s "$dir/libreins_core.a" && -s "$dir/include/module.modulemap" && -s "$dir/include/reins_coreFFI.h" &&
     "$(cat "$dir/.hash" 2>/dev/null)" == "$hash" ]]
}

# Unpacks a core archive into target/ios-core/<platform> and checks the committed bindings match it.
install_archive() {
  local platform="$1" archive="$2" dir stage
  dir="$(out_dir "$platform")"
  mkdir -p "$(dirname "$dir")"
  stage="$(mktemp -d "$(dirname "$dir")/.fetch.XXXXXX")"
  tar -xzf "$archive" -C "$stage"
  for f in libreins_core.a include/module.modulemap include/reins_coreFFI.h reins_core.swift .hash; do
    [[ -s "$stage/$f" ]] || { rm -rf "$stage"; die "archive $archive lacks $f"; }
  done
  if ! cmp -s "$stage/reins_core.swift" "$ROOT/ios/Core/Generated/reins_core.swift"; then
    echo "core-cache: warning: ios/Core/Generated/reins_core.swift differs from the prebuilt core's bindings;" \
      "run ios/scripts/build-core.sh and commit its output" >&2
  fi
  rm -rf "$dir"
  mv "$stage" "$dir"
}

fetch() {
  local platform="$1" name tmp url
  name="$(asset_name "$platform")"
  url="https://github.com/$REPO/releases/download/$TAG/$name"
  mkdir -p "$ROOT/target/ios-core"
  tmp="$(mktemp "$ROOT/target/ios-core/.dl.XXXXXX")"
  # The repository is public: no token needed, so the Mac and CI's read-only jobs fetch the same way.
  if curl -fsSL --retry 3 --retry-delay 2 -o "$tmp" "$url"; then
    install_archive "$platform" "$tmp"
    rm -f "$tmp"
    echo "core-cache: fetched $name"
    return 0
  fi
  rm -f "$tmp"
  echo "core-cache: no prebuilt core $name" >&2
  return 1
}

pack() {
  local platform="$1" file="$2" dir
  dir="$(out_dir "$platform")"
  [[ -s "$dir/libreins_core.a" ]] || die "no built core in $dir"
  cp "$ROOT/ios/Core/Generated/reins_core.swift" "$dir/reins_core.swift"
  content_hash > "$dir/.hash"
  tar -czf "$file" -C "$dir" libreins_core.a include/module.modulemap include/reins_coreFFI.h reins_core.swift .hash
}

publish() {
  local platform="$1" file="${2:-}" name keep
  name="$(asset_name "$platform")"
  if [[ -z "$file" ]]; then
    file="$ROOT/target/ios-core/$name"
    pack "$platform" "$file"
  fi
  [[ -s "$file" ]] || die "nothing to publish at $file"
  if ! gh release view "$TAG" -R "$REPO" >/dev/null 2>&1; then
    # A prerelease whose tag matches no vX.Y.Z: scripts/next-version.sh and releases/latest never see it.
    gh release create "$TAG" -R "$REPO" --prerelease --latest=false --title "iOS core cache (not a release)" \
      --notes "Prebuilt Rust cores for iOS Debug builds, one per content hash of the core's sources. Written by ios/scripts/core-cache.sh; not a release." ||
      gh release view "$TAG" -R "$REPO" >/dev/null
  fi
  local upload="$file"
  if [[ "$(basename "$file")" != "$name" ]]; then
    upload="$(dirname "$file")/$name"
    cp "$file" "$upload"
  fi
  gh release upload "$TAG" "$upload" -R "$REPO" --clobber
  echo "core-cache: published $name"
  # Prune: keep the newest REINS_CORE_KEEP assets of this platform.
  keep="${REINS_CORE_KEEP:-12}"
  gh api "repos/$REPO/releases/tags/$TAG" --jq \
    ".assets | map(select(.name | endswith(\"-$platform.tar.gz\"))) | sort_by(.created_at) | reverse | .[$keep:] | .[].id" |
    while IFS= read -r id; do
      [[ -n "$id" ]] && gh api -X DELETE "repos/$REPO/releases/assets/$id" >/dev/null && echo "core-cache: pruned asset $id"
    done || true
}

ensure() {
  local platform="$1"
  if stamp_ok "$platform"; then
    echo "core-cache: target/ios-core/$platform is current ($(content_hash))"
    return 0
  fi
  fetch "$platform" && return 0
  echo "core-cache: building the core for $platform (no prebuilt for $(content_hash))"
  # build-core.sh would otherwise reuse whatever library is there.
  REINS_SKIP_CORE= CONFIGURATION=Debug "$ROOT/ios/scripts/build-core.sh" "$platform"
  local dir
  dir="$(out_dir "$platform")"
  cp "$ROOT/ios/Core/Generated/reins_core.swift" "$dir/reins_core.swift"
  content_hash > "$dir/.hash"
  if [[ "${REINS_CORE_PUBLISH:-}" == 1 ]]; then publish "$platform" || echo "core-cache: publish failed (kept the local build)" >&2; fi
}

cmd="${1:-}"
case "$cmd" in
  hash) content_hash ;;
  asset) platform_ok "${2:-}"; asset_name "$2" ;;
  fetch) platform_ok "${2:-}"; fetch "$2" ;;
  ensure) platform_ok "${2:-}"; ensure "$2" ;;
  install) platform_ok "${2:-}"; [[ -s "${3:-}" ]] || die "install needs an archive"; install_archive "$2" "$3" ;;
  pack) platform_ok "${2:-}"; [[ -n "${3:-}" ]] || die "pack needs a file"; pack "$2" "$3" ;;
  publish) platform_ok "${2:-}"; publish "$2" "${3:-}" ;;
  *) sed -n '2,20p' "$0" >&2; exit 2 ;;
esac
