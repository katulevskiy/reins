#!/bin/bash
# Live onboarding test: the real iOS app (real Rust core, no -demo) on a simulator creates an account on a real local
# Reins server, becomes the approval device and pairs a computer by the code it shows.
#
#   ios/scripts/live-onboarding.sh [simulator name or UDID] [dir]      (defaults "Reins e2e", /tmp/reins-onboarding)
#
# The host half (`cargo run -p rewarden-e2e --example ios_onboarding`) starts the server with no account, waits for the
# phone to have created it (<dir>/created), then plays the desktop app: it starts the device flow, prints the pairing
# code and the number the computer shows, and makes an authenticated call once the phone approved. The phone half
# (ReinsUITests/LiveOnboardingUITests) creates the account through "Use another server", allows notifications, opens
# the code as a reins://pair link (the simulator has no camera), taps the number and approves with Face ID. Face ID is
# enrolled on the simulator and matched by this script whenever the test asks (it writes <dir>/faceid). Screenshots and
# logs end up in <dir>. Set CARGO_TARGET_DIR to reuse a server build.
set -euo pipefail
export DEVELOPER_DIR="${DEVELOPER_DIR:-/Applications/Xcode-27.1-beta.app/Contents/Developer}"
SIM="${1:-Reins e2e}"
OUT="${2:-/tmp/reins-onboarding}"
ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
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

# A fresh phone: no app (so notifications are asked again), no keychain items, Face ID enrolled.
xcrun simctl boot "$UDID" 2>/dev/null || true
xcrun simctl bootstatus "$UDID" -b >/dev/null
xcrun simctl uninstall "$UDID" com.reins2fa.app 2>/dev/null || true
xcrun simctl keychain "$UDID" reset >/dev/null 2>&1 || true
xcrun simctl spawn "$UDID" notifyutil -s com.apple.BiometricKit.enrollmentChanged 1
xcrun simctl spawn "$UDID" notifyutil -p com.apple.BiometricKit.enrollmentChanged

echo "building the server and the host half"
(cd "$ROOT" && cargo build -q --features sqlite --bin vaultwarden && cargo build -q -p rewarden-e2e --example ios_onboarding)
# Run the example binary itself: under `cargo run`, the server build it starts sees cargo's variables and rebuilds.
(cd "$ROOT" && exec "${CARGO_TARGET_DIR:-$ROOT/target}/debug/examples/ios_onboarding" "$OUT") >"$OUT/host.log" 2>&1 &
HOST=$!
WATCH="" SERVER=""
cleanup() {
  touch "$OUT/done"
  [[ -n "$WATCH" ]] && kill "$WATCH" 2>/dev/null
  sleep 1
  if kill -0 "$HOST" 2>/dev/null; then
    # Still running (a failed run, or keeping the server up): stop it, and the server it started, which a kill
    # leaves running.
    kill "$HOST" 2>/dev/null || true
    [[ -n "$SERVER" ]] && lsof -ti "tcp:${SERVER##*:}" -sTCP:LISTEN 2>/dev/null | xargs kill 2>/dev/null || true
  fi
}
trap cleanup EXIT
wait_for "$OUT/host.log" '^PASSWORD ' 900
SERVER=$(field SERVER) EMAIL=$(field EMAIL)
PASSWORD=$(awk '/^PASSWORD /{sub(/^PASSWORD /, ""); print; exit}' "$OUT/host.log")
echo "server $SERVER, new account $EMAIL"

# Face ID: while the test waits on an approval (<dir>/faceid exists), keep offering a matching face.
(while kill -0 "$HOST" 2>/dev/null; do
  if [[ -e "$OUT/faceid" ]]; then xcrun simctl spawn "$UDID" notifyutil -p com.apple.BiometricKit_Sim.pearl.match; fi
  sleep 1
done) &
WATCH=$!

# What the "computer" shows once the phone has the account: the code (for the link) and the number to tap.
(wait_for "$OUT/host.log" '^CONFIRM ' 900 >/dev/null \
   && field USER_CODE >"$OUT/usercode.tmp" && field CONFIRM >"$OUT/confirm.tmp" \
   && mv "$OUT/confirm.tmp" "$OUT/confirm" && mv "$OUT/usercode.tmp" "$OUT/usercode"
 wait_for "$OUT/host.log" '^ALL_DONE' 600 >/dev/null && touch "$OUT/host-done") &

echo "running the UI test on $UDID"
set +e
(cd "$ROOT/ios" && TEST_RUNNER_REINS_LIVE_DIR="$OUT" TEST_RUNNER_REINS_LIVE_SERVER="$SERVER" \
  TEST_RUNNER_REINS_LIVE_EMAIL="$EMAIL" TEST_RUNNER_REINS_LIVE_PASSWORD="$PASSWORD" \
  REINS_SKIP_CORE=1 xcodebuild test -project Reins.xcodeproj -scheme Reins -destination "id=$UDID" \
  -derivedDataPath "/tmp/reins-dd-$UDID" -only-testing:ReinsUITests/LiveOnboardingUITests \
  -collect-test-diagnostics never -resultBundlePath "$OUT/LiveOnboarding.xcresult" >"$OUT/xcodebuild.log" 2>&1)
TEST=$?
set -e
grep -E "^(SERVER|EMAIL|REGISTERED|QR|FINGERPRINT|USER_CODE|CONFIRM|PAIRED|TOOLS|ALL_DONE|FAILED)|panicked|error" "$OUT/host.log" || true
grep -E "Test Case|error:|failed|passed" "$OUT/xcodebuild.log" | tail -20
if [[ $TEST -eq 0 ]] && grep -q '^ALL_DONE' "$OUT/host.log"; then
  echo "LIVE ONBOARDING: OK (screenshots in $OUT)"
else
  echo "LIVE ONBOARDING: FAILED (logs in $OUT)"; exit 1
fi
