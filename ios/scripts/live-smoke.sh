#!/bin/bash
# Live smoke test: the real iOS app (real Rust core, no -demo) on a simulator against a real local Reins server.
#
#   ios/scripts/live-smoke.sh [simulator name or UDID]      (default "Reins e2e")
#
# The host half (`cargo run -p rewarden-e2e --example ios_smoke`) starts the server with an account and a small MCP
# server, then plays the AI: it pairs once the phone registered, and calls the MCP tools once the phone added it.
# The phone half (ReinsUITests/LiveServerUITests) signs in, picks the pairing code, adds the MCP server, approves one
# call once and allows the next for a while, then looks at Activity, Grants and Settings. Face ID is enrolled on the
# simulator and matched by this script whenever the test asks (it writes <dir>/faceid). Screenshots and logs end up in
# /tmp/reins-live/. Set CARGO_TARGET_DIR to reuse a server build.
set -euo pipefail
export DEVELOPER_DIR="${DEVELOPER_DIR:-/Applications/Xcode-27.1-beta.app/Contents/Developer}"
SIM="${1:-Reins e2e}"
ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
OUT=/tmp/reins-live
rm -rf "$OUT"; mkdir -p "$OUT"
export PATH="$HOME/.cargo/bin:$PATH"

UDID="$(xcrun simctl list devices -j | python3 -c "
import json,sys
want=sys.argv[1]
for rt,devs in json.load(sys.stdin)['devices'].items():
    for d in devs:
        if d['udid']==want or d['name']==want: print(d['udid']); sys.exit()
" "$SIM")"
[[ -n "$UDID" ]] || { echo "no simulator $SIM" >&2; exit 1; }

wait_for() { # file pattern seconds
  for _ in $(seq 1 "$3"); do grep -q "$2" "$1" 2>/dev/null && return 0; sleep 1; done
  echo "timed out waiting for '$2' in $1" >&2; tail -20 "$1" >&2; return 1
}
field() { awk -v k="$1" '$1==k{print $2; exit}' "$OUT/host.log"; }

# A fresh phone: no app, no keychain items, Face ID enrolled.
xcrun simctl boot "$UDID" 2>/dev/null || true
xcrun simctl bootstatus "$UDID" -b >/dev/null
xcrun simctl uninstall "$UDID" com.reins2fa.app 2>/dev/null || true
xcrun simctl keychain "$UDID" reset >/dev/null 2>&1 || true
xcrun simctl spawn "$UDID" notifyutil -s com.apple.BiometricKit.enrollmentChanged 1
xcrun simctl spawn "$UDID" notifyutil -p com.apple.BiometricKit.enrollmentChanged

echo "building the server and the host half"
(cd "$ROOT" && cargo build -q --features sqlite --bin vaultwarden && cargo build -q -p rewarden-e2e --example ios_smoke)
# Run the example binary itself: under `cargo run`, the server build it starts sees cargo's variables and rebuilds.
(cd "$ROOT" && exec "${CARGO_TARGET_DIR:-$ROOT/target}/debug/examples/ios_smoke" "$OUT") >"$OUT/host.log" 2>&1 &
HOST=$!
WATCH="" SERVER=""
cleanup() {
  touch "$OUT/done"
  [[ -n "$WATCH" ]] && kill "$WATCH" 2>/dev/null
  sleep 1
  if kill -0 "$HOST" 2>/dev/null; then
    # Still waiting on the phone (a failed run): stop it, and the server it started, which a kill leaves running.
    kill "$HOST" 2>/dev/null || true
    [[ -n "$SERVER" ]] && lsof -ti "tcp:${SERVER##*:}" -sTCP:LISTEN 2>/dev/null | xargs kill 2>/dev/null || true
  fi
}
trap cleanup EXIT
wait_for "$OUT/host.log" '^MCP ' 900
SERVER=$(field SERVER) EMAIL=$(field EMAIL) MCP=$(field MCP)
PASSWORD=$(awk '/^PASSWORD /{sub(/^PASSWORD /, ""); print; exit}' "$OUT/host.log")
echo "server $SERVER, account $EMAIL, MCP server $MCP"

# Face ID: while the test waits on an approval (<dir>/faceid exists), keep offering a matching face.
(while kill -0 "$HOST" 2>/dev/null; do
  if [[ -e "$OUT/faceid" ]]; then xcrun simctl spawn "$UDID" notifyutil -p com.apple.BiometricKit_Sim.pearl.match; fi
  sleep 1
done) &
WATCH=$!

# The pairing code the "browser" shows, for the test to pick.
(wait_for "$OUT/host.log" '^CODE ' 600 >/dev/null && field CODE >"$OUT/code.tmp" && mv "$OUT/code.tmp" "$OUT/code"
 wait_for "$OUT/host.log" '^ALL_DONE' 600 >/dev/null && touch "$OUT/host-done") &

echo "running the UI test on $UDID"
set +e
(cd "$ROOT/ios" && TEST_RUNNER_REINS_LIVE_DIR="$OUT" TEST_RUNNER_REINS_LIVE_SERVER="$SERVER" \
  TEST_RUNNER_REINS_LIVE_EMAIL="$EMAIL" TEST_RUNNER_REINS_LIVE_PASSWORD="$PASSWORD" TEST_RUNNER_REINS_LIVE_MCP="$MCP" \
  REINS_SKIP_CORE=1 xcodebuild test -project Reins.xcodeproj -scheme Reins -destination "id=$UDID" \
  -derivedDataPath "/tmp/reins-dd-$UDID" -only-testing:ReinsUITests/LiveServerUITests \
  -collect-test-diagnostics never -resultBundlePath "$OUT/LiveServer.xcresult" >"$OUT/xcodebuild.log" 2>&1)
TEST=$?
set -e
grep -E "^(SERVER|EMAIL|CODE|REGISTERED|PAIRED|MCP_READY|TOOL_RESULT|ALL_DONE)|panicked|error" "$OUT/host.log" || true
grep -E "Test Case|error:|failed|passed" "$OUT/xcodebuild.log" | tail -20
if [[ $TEST -eq 0 ]] && grep -q '^ALL_DONE' "$OUT/host.log"; then
  echo "LIVE SMOKE: OK (screenshots in $OUT)"
else
  echo "LIVE SMOKE: FAILED (logs in $OUT)"; exit 1
fi
