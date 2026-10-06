#!/usr/bin/env bash
# Release workers build each ABI and the host-generated bindings independently.
# These outputs contain no release version or signing material; cache them by exact source content.
set -euo pipefail
cd "$(dirname "${BASH_SOURCE[0]}")/.."
kind="${1:?expected arm64-v8a, x86_64 or bindings}"
out="${2:?output directory}"
profile="${3:-release}"
[[ "$profile" == release || "$profile" == dev ]] || { echo "Unexpected native profile: $profile" >&2; exit 1; }
mkdir -p "$out"
case "$kind" in
arm64-v8a | x86_64)
    if [[ -n "${ANDROID_HOME:-}" ]]; then
        ndk="$(sed -n 's/.*reinsNdkVersion = "\([^"]*\)".*/\1/p' android/app/build.gradle.kts)"
        export ANDROID_NDK_HOME="$ANDROID_HOME/ndk/$ndk"
    fi
    export CARGO_ENCODED_RUSTFLAGS="-Clink-arg=-Wl,-z,max-page-size=16384"
    cargo ndk -t "$kind" --platform 31 -o "$out/jni" build --profile "$profile" -p reins-core --lib --locked
    test -s "$out/jni/$kind/libreins_core.so"
    ;;
bindings)
    # UniFFI's metadata is independent of the Android target; keep host library symbols.
    cargo build --locked -p reins-core --lib --features bindgen
    host_lib=libreins_core.so
    [[ "$(uname -s)" == Darwin ]] && host_lib=libreins_core.dylib
    cargo run --locked -q -p reins-core --features bindgen --bin uniffi-bindgen -- \
        generate --library "${CARGO_TARGET_DIR:-target}/debug/$host_lib" \
        --language kotlin --no-format --out-dir "$out/kotlin"
    test -s "$out/kotlin/dev/reins/core/reins_core.kt"
    ;;
*) echo "Unknown native build kind: $kind" >&2; exit 1 ;;
esac
