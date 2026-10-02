#!/usr/bin/env bash
# On-device smoke test: the real server + a simulated AI client on this machine, the real phone core on an emulator.
# Usage: android/scripts/device-smoke.sh [adb-serial]   (default emulator-5554)
set -euo pipefail

SERIAL="${1:-emulator-5554}"
ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
export ANDROID_HOME="${ANDROID_HOME:-$HOME/Android/Sdk}" ANDROID_SERIAL="$SERIAL" PATH="$HOME/.cargo/bin:$PATH"
OUT="$(mktemp -d)"; echo "logs in $OUT"
GO=/tmp/rewarden-smoke-go
DONE=/tmp/rewarden-smoke-done
rm -f "$GO" "$DONE"
adb -s "$SERIAL" shell run-as dev.rewarden.android rm -f files/live-code 2>/dev/null || true

cd "$ROOT"
cargo build -q -p rewarden-e2e --example device_smoke
cargo run -q -p rewarden-e2e --example device_smoke >"$OUT/host.log" 2>&1 &
HOST=$!
trap 'kill $HOST 2>/dev/null || true' EXIT

wait_for() { # file pattern seconds
  for _ in $(seq 1 "$3"); do grep -q "$2" "$1" 2>/dev/null && return 0; sleep 1; done
  echo "timed out waiting for '$2' in $1" >&2; cat "$1" >&2; return 1
}

wait_for "$OUT/host.log" EMAIL 120
SERVER=$(awk '/^SERVER/{print $2}' "$OUT/host.log")
PORT=${SERVER##*:}
adb -s "$SERIAL" reverse "tcp:$PORT" "tcp:$PORT" >/dev/null

(cd android && ./gradlew connectedFullDebugAndroidTest -Prewarden.rustProfile="${RUST_PROFILE:-release-low}" \
  -Pandroid.testInstrumentationRunnerArguments.class=dev.rewarden.android.LiveServerTest \
  -Pandroid.testInstrumentationRunnerArguments.live.server="$SERVER" \
  -Pandroid.testInstrumentationRunnerArguments.live.email=phone@example.com >"$OUT/gradle.log" 2>&1) &
GRADLE=$!

# The AI starts connecting once the phone registered as the approval device.
SERVER_DIR=$(ls -dt /tmp/rewarden-e2e-* | head -1)
wait_for "$SERVER_DIR/server.log" "PUT /rewarden/api/device" 300
touch "$GO"
wait_for "$OUT/host.log" CODE 60
CODE=$(awk '/^CODE/{print $2}' "$OUT/host.log")
adb -s "$SERIAL" shell "run-as dev.rewarden.android sh -c 'echo $CODE > files/live-code'"

wait_for "$OUT/host.log" TOOL_RESULT 90
touch "$DONE"
wait "$GRADLE" && echo "phone side: OK" || { echo "phone side FAILED"; tail -30 "$OUT/gradle.log"; exit 1; }
cat "$OUT/host.log"
