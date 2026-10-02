#!/bin/bash
# Builds the Rust phone core (crates/rewarden-core) for the active Xcode platform and generates its Swift bindings.
# Run by the "Rust core" build phase of the Reins and ReinsNotifications targets; also runnable by hand (defaults to
# the simulator):
#
#   ios/scripts/build-core.sh [iphonesimulator|iphoneos]
#
# Outputs (target/ios-core/<platform>/):
#   librewarden_core.a          linked through LIBRARY_SEARCH_PATHS
#   include/module.modulemap    `import rewarden_coreFFI` (SWIFT_INCLUDE_PATHS)
# and refreshes ios/Core/Generated/rewarden_core.swift, which is committed so Xcode's synchronized folder always
# sees it. REINS_SKIP_CORE=1 reuses the last build while iterating on Swift only.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
PLATFORM="${1:-${PLATFORM_NAME:-iphonesimulator}}"
case "$PLATFORM" in
  iphonesimulator) TARGET=aarch64-apple-ios-sim ;;
  iphoneos) TARGET=aarch64-apple-ios ;;
  *) echo "error: unsupported platform $PLATFORM" >&2; exit 1 ;;
esac
# Debug builds still optimise the core (argon2 and the tokenizer are unusable at opt-level 0); Release is fat LTO.
PROFILE=release-low
if [[ "${CONFIGURATION:-Debug}" == "Release" ]]; then PROFILE=release; fi

OUT="$ROOT/target/ios-core/$PLATFORM"
mkdir -p "$OUT/include"

if [[ "${REINS_SKIP_CORE:-}" == "1" && -f "$OUT/librewarden_core.a" ]]; then
  echo "note: REINS_SKIP_CORE=1, reusing $OUT/librewarden_core.a"
  exit 0
fi

# Two targets (the app and the notification extension) run this phase; one build at a time.
LOCK="$ROOT/target/ios-core/.lock"
while ! mkdir "$LOCK" 2>/dev/null; do
  if [[ -n "$(find "$LOCK" -maxdepth 0 -mmin +30 2>/dev/null)" ]]; then rmdir "$LOCK" 2>/dev/null || true; fi
  sleep 1
done
trap 'rmdir "$LOCK" 2>/dev/null || true' EXIT

# Xcode exports SDKROOT and deployment variables for the app's SDK; host build scripts (proc macros, build.rs) must not
# see them, so cargo runs in a clean environment with the toolchain rustup pins (rust-toolchain.toml).
run_cargo() {
  env -i HOME="$HOME" USER="${USER:-}" TERM="${TERM:-dumb}" \
    PATH="$HOME/.cargo/bin:/usr/local/bin:/opt/homebrew/bin:/usr/bin:/bin:/usr/sbin:/sbin" \
    IPHONEOS_DEPLOYMENT_TARGET=26.0 \
    CARGO_TARGET_DIR="$ROOT/target" \
    cargo "$@"
}

cd "$ROOT"
rustup target add "$TARGET" >/dev/null 2>&1 || true
# The crate builds a cdylib for Android; iOS links a static library, asked for here so other builds stay as they are.
run_cargo rustc --locked -q -p rewarden-core --lib --crate-type staticlib --profile "$PROFILE" --target "$TARGET"
run_cargo build --locked -q -p rewarden-core --bin uniffi-bindgen --features bindgen --profile release-low

LIB="$ROOT/target/$TARGET/$PROFILE/librewarden_core.a"
cp -p "$LIB" "$OUT/librewarden_core.a.tmp" && mv "$OUT/librewarden_core.a.tmp" "$OUT/librewarden_core.a"

GEN="$OUT/gen"
rm -rf "$GEN"
"$ROOT/target/release-low/uniffi-bindgen" generate --library "$LIB" --language swift --out-dir "$GEN" >/dev/null
cmp -s "$GEN/rewarden_coreFFI.h" "$OUT/include/rewarden_coreFFI.h" || cp "$GEN/rewarden_coreFFI.h" "$OUT/include/rewarden_coreFFI.h"
cmp -s "$GEN/rewarden_coreFFI.modulemap" "$OUT/include/module.modulemap" ||
  cp "$GEN/rewarden_coreFFI.modulemap" "$OUT/include/module.modulemap"
# Only touch the Swift file when it changed, so Xcode does not recompile it.
SWIFT_OUT="$ROOT/ios/Core/Generated/rewarden_core.swift"
mkdir -p "$(dirname "$SWIFT_OUT")"
cmp -s "$GEN/rewarden_core.swift" "$SWIFT_OUT" || cp "$GEN/rewarden_core.swift" "$SWIFT_OUT"
