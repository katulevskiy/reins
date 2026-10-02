#!/bin/bash
# Builds the app, installs it on a simulator and launches it, optionally taking a screenshot.
#
#   ios/scripts/sim.sh <simulator name or UDID> [--shot out.png] [--wait SECS] [--] [launch args, e.g. -demo]
#
# Uses Xcode 27.1 (UIHinge, the iPhone Duo runtime), a derived-data folder per simulator, and the last Rust core
# build (REINS_SKIP_CORE=1; run ios/scripts/build-core.sh after changing the core). Creates nothing: make the
# simulator first, e.g. xcrun simctl create "Reins 18 Pro Max" com.apple.CoreSimulator.SimDeviceType.iPhone-18-Pro-Max
# com.apple.CoreSimulator.SimRuntime.iOS-27-2
set -euo pipefail
export DEVELOPER_DIR="${DEVELOPER_DIR:-/Applications/Xcode-27.1-beta.app/Contents/Developer}"
IOS="$(cd "$(dirname "$0")/.." && pwd)"
SIM="$1"; shift
SHOT="" WAIT=4
while [[ $# -gt 0 ]]; do
  case "$1" in
    --shot) SHOT="$2"; shift 2 ;;
    --wait) WAIT="$2"; shift 2 ;;
    --) shift; break ;;
    *) break ;;
  esac
done
UDID="$(xcrun simctl list devices -j | python3 -c "
import json,sys
want=sys.argv[1]
for rt,devs in json.load(sys.stdin)['devices'].items():
    for d in devs:
        if d['udid']==want or d['name']==want: print(d['udid']); sys.exit()
" "$SIM")"
[[ -n "$UDID" ]] || { echo "no simulator $SIM" >&2; exit 1; }
DD="/tmp/reins-dd-$UDID"
REINS_SKIP_CORE="${REINS_SKIP_CORE:-1}" xcodebuild -project "$IOS/Reins.xcodeproj" -scheme Reins \
  -destination "id=$UDID" -derivedDataPath "$DD" build -quiet 2>&1 | grep -E "error:|warning: .*deprecated" | grep -v "^$" | sort -u || true
APP="$DD/Build/Products/Debug-iphonesimulator/Reins.app"
[[ -d "$APP" ]] || { echo "build failed" >&2; exit 1; }
xcrun simctl boot "$UDID" 2>/dev/null || true
xcrun simctl bootstatus "$UDID" -b >/dev/null
xcrun simctl terminate "$UDID" dev.rewarden.ios 2>/dev/null || true
xcrun simctl install "$UDID" "$APP"
xcrun simctl launch "$UDID" dev.rewarden.ios "$@" >/dev/null
if [[ -n "$SHOT" ]]; then
  sleep "$WAIT"
  xcrun simctl io "$UDID" screenshot "$SHOT" >/dev/null 2>&1
  echo "$SHOT"
fi
