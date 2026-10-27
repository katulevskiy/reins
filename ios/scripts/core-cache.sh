#!/bin/bash
# The prebuilt Rust core for iOS (the static library, its C header and module map, and the Swift bindings), one
# build per content hash of the core's sources, published by .github/workflows/ios-core.yml on main as assets of the
# rolling `ios-core-cache` prerelease, each with a build provenance attestation.
#
#   ios/scripts/core-cache.sh hash                 the content hash (16 hex digits)
#   ios/scripts/core-cache.sh asset <variant>      the asset's file name for this tree
#   ios/scripts/core-cache.sh fetch <variant>      download, verify and install it in target/ios-core/<platform>;
#                                                  exit 1 when there is none or it does not verify
#   ios/scripts/core-cache.sh ensure <variant>     the stamp matches, else fetch, else build-core.sh (stamped)
#   ios/scripts/core-cache.sh install <variant> <file> unpack an archive this same run built (an artifact)
#   ios/scripts/core-cache.sh pack <variant> <file>    tar the built core (for ios-core.yml)
#
# Trust: fetch installs an asset only when `gh attestation verify` shows it was attested by ios-core.yml running on
# main (the certificate's workflow identity and source ref, on a GitHub-hosted runner). Without gh, or when the
# check fails, it installs nothing and ensure builds the core here instead. Publishing lives in ios-core.yml only,
# never replaces an asset, and only ever publishes a build of main's own commit. A core whose bindings do not match
# ios/Core/Generated/reins_core.swift is refused (stale committed bindings: run build-core.sh and commit them).
#
# <variant> is iphonesimulator or iphoneos (Debug: build-core.sh's release-low profile), or iphoneos-release (Release:
# the workspace's release profile, which the TestFlight archive in release.yml links). Both iphoneos variants install
# into target/ios-core/iphoneos, where Xcode looks; the archive records its profile (.profile) and the iOS SDK it was
# built with (.sdk), and the stamp tells the two apart. The hash covers what the core build reads: the three crates,
# the workspace Cargo.toml (profiles) and Cargo.lock, rust-toolchain.toml, .cargo/, uniffi.toml and build-core.sh.
# It hashes the working tree (git hash-object), so a local edit to the core changes it before it is committed. An
# archive is installed only when the hash it carries is this tree's.
#
# Why a release asset and not an Actions cache: caches are scoped to the branch that wrote them (plus main), evicted
# after 7 days unused or when the repository passes 10 GB, and have no download API for the Mac or this PC. A
# release asset is public, permanent until pruned, downloadable with curl from anywhere, and keyed by its name.
#
# Environment: REINS_CORE_BUILD_ONLY=1 makes `ensure` build even when a prebuilt
# core exists (ios-core.yml, whose build is the one published).
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
# Fixed, not GH_REPO: only this repository's main-branch builds are trusted.
REPO=katulevskiy/reins
TAG=ios-core-cache  # also in .github/workflows/ios-core.yml
FORMAT=v1
PROFILE=release-low
INPUTS=(
  crates/reins-core crates/reins-proto crates/reins-policy
  Cargo.toml Cargo.lock rust-toolchain.toml .cargo ios/scripts/build-core.sh
)

die() { echo "core-cache: $*" >&2; exit 1; }

variant_ok() {
  case "${1:-}" in
    iphonesimulator|iphoneos|iphoneos-release) ;;
    *) die "variant must be iphonesimulator, iphoneos or iphoneos-release (got '${1:-}')" ;;
  esac
}

# The Xcode platform (and folder) of a variant, its cargo profile, and the configuration build-core.sh builds it in.
platform_of() { echo "${1%-release}"; }
profile_of() { if [[ "$1" == *-release ]]; then echo release; else echo "$PROFILE"; fi; }
configuration_of() { if [[ "$1" == *-release ]]; then echo Release; else echo Debug; fi; }

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

out_dir() { echo "$ROOT/target/ios-core/$(platform_of "$1")"; }

# The profile a built core folder holds (cores from before .profile are release-low).
dir_profile() { cat "$1/.profile" 2>/dev/null || echo "$PROFILE"; }

stamp_ok() {
  local dir hash
  dir="$(out_dir "$1")"
  hash="$(content_hash)"
  [[ -s "$dir/libreins_core.a" && -s "$dir/include/module.modulemap" && -s "$dir/include/reins_coreFFI.h" &&
     "$(cat "$dir/.hash" 2>/dev/null)" == "$hash" && "$(dir_profile "$dir")" == "$(profile_of "$1")" ]]
}

# Unpacks a core archive into target/ios-core/<platform>, after checking it was built from this tree's sources, in the
# variant's profile, and that the committed bindings match it.
install_archive() {
  local variant="$1" archive="$2" dir stage held
  dir="$(out_dir "$variant")"
  mkdir -p "$(dirname "$dir")"
  stage="$(mktemp -d "$(dirname "$dir")/.fetch.XXXXXX")"
  tar -xzf "$archive" -C "$stage"
  for f in libreins_core.a include/module.modulemap include/reins_coreFFI.h reins_core.swift .hash; do
    [[ -s "$stage/$f" ]] || { rm -rf "$stage"; die "archive $archive lacks $f"; }
  done
  if [[ "$(cat "$stage/.hash")" != "$(content_hash)" ]]; then
    rm -rf "$stage"
    die "archive $archive was built from other sources, not $(content_hash)"
  fi
  held="$(dir_profile "$stage")"
  if [[ "$held" != "$(profile_of "$variant")" ]]; then
    rm -rf "$stage"
    die "archive $archive holds a $held core, not $(profile_of "$variant")"
  fi
  if ! cmp -s "$stage/reins_core.swift" "$ROOT/ios/Core/Generated/reins_core.swift"; then
    rm -rf "$stage"
    die "ios/Core/Generated/reins_core.swift does not match the bindings of the core built from these sources:" \
      "run ios/scripts/build-core.sh and commit its output"
  fi
  rm -rf "$dir"
  mv "$stage" "$dir"
}

fetch() {
  local variant="$1" name tmp url
  name="$(asset_name "$variant")"
  url="https://github.com/$REPO/releases/download/$TAG/$name"
  mkdir -p "$ROOT/target/ios-core"
  tmp="$(mktemp "$ROOT/target/ios-core/.dl.XXXXXX")"
  # The repository is public: no token needed to download.
  if ! curl -fsSL --retry 3 --retry-delay 2 -o "$tmp" "$url"; then
    rm -f "$tmp"
    echo "core-cache: no prebuilt core $name" >&2
    return 1
  fi
  if ! verify "$tmp"; then
    rm -f "$tmp"
    echo "core-cache: $name could not be traced to ios-core.yml on main; not installing it" >&2
    return 1
  fi
  install_archive "$variant" "$tmp"
  rm -f "$tmp"
  echo "core-cache: fetched and verified $name"
}

# The asset's provenance: attested by ios-core.yml itself, running for refs/heads/main on a GitHub-hosted runner.
verify() {
  command -v gh >/dev/null 2>&1 || { echo "core-cache: gh is not installed, so the download cannot be verified" >&2; return 1; }
  gh attestation verify "$1" --repo "$REPO" \
    --cert-identity "https://github.com/$REPO/.github/workflows/ios-core.yml@refs/heads/main" \
    --source-ref refs/heads/main \
    --deny-self-hosted-runners >/dev/null
}

pack() {
  local variant="$1" file="$2" dir files
  dir="$(out_dir "$variant")"
  [[ -s "$dir/libreins_core.a" ]] || die "no built core in $dir"
  [[ "$(dir_profile "$dir")" == "$(profile_of "$variant")" ]] || die "$dir holds a $(dir_profile "$dir") core"
  cp "$ROOT/ios/Core/Generated/reins_core.swift" "$dir/reins_core.swift"
  content_hash > "$dir/.hash"
  profile_of "$variant" > "$dir/.profile"
  files=(libreins_core.a include/module.modulemap include/reins_coreFFI.h reins_core.swift .hash .profile)
  # The SDK the C parts were compiled against: the TestFlight job takes a core only from its own Xcode's SDK.
  if command -v xcrun >/dev/null 2>&1 && xcrun --sdk "$(platform_of "$variant")" --show-sdk-version > "$dir/.sdk"; then
    files+=(.sdk)
  else
    rm -f "$dir/.sdk"
  fi
  tar -czf "$file" -C "$dir" "${files[@]}"
}

ensure() {
  local variant="$1"
  if stamp_ok "$variant"; then
    echo "core-cache: target/ios-core/$(platform_of "$variant") is current ($(content_hash), $(profile_of "$variant"))"
    return 0
  fi
  if [[ "${REINS_CORE_BUILD_ONLY:-}" != 1 ]]; then
    fetch "$variant" && return 0
  fi
  echo "core-cache: building the core for $variant (no prebuilt for $(content_hash))"
  local dir
  dir="$(out_dir "$variant")"
  # A stamp of another profile must not outlive this build.
  rm -f "$dir/.hash" "$dir/.profile" "$dir/.sdk"
  # build-core.sh would otherwise reuse whatever library is there.
  REINS_SKIP_CORE='' CONFIGURATION="$(configuration_of "$variant")" "$ROOT/ios/scripts/build-core.sh" "$(platform_of "$variant")"
  cp "$ROOT/ios/Core/Generated/reins_core.swift" "$dir/reins_core.swift"
  content_hash > "$dir/.hash"
  profile_of "$variant" > "$dir/.profile"
}

cmd="${1:-}"
case "$cmd" in
  hash) content_hash ;;
  asset) variant_ok "${2:-}"; asset_name "$2" ;;
  fetch) variant_ok "${2:-}"; fetch "$2" ;;
  ensure) variant_ok "${2:-}"; ensure "$2" ;;
  install) variant_ok "${2:-}"; [[ -s "${3:-}" ]] || die "install needs an archive"; install_archive "$2" "$3" ;;
  pack) variant_ok "${2:-}"; [[ -n "${3:-}" ]] || die "pack needs a file"; pack "$2" "$3" ;;
  *) sed -n '2,33p' "$0" >&2; exit 2 ;;
esac
