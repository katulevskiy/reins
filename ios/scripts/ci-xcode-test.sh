#!/bin/bash
# One iOS test slice on a fresh simulator. Used by the iOS jobs in .github/workflows/ci.yml.
#
#   ONLY   comma-separated -only-testing identifiers (required), e.g. ReinsTests or ReinsUITests/ActivityFlowTests
#   SKIP   comma-separated -skip-testing identifiers (optional)
#
# Expects DEVELOPER_DIR, GITHUB_WORKSPACE and RUNNER_TEMP. The Rust core is the one ios-core already built
# (target/ios-core/iphonesimulator, REINS_SKIP_CORE=1). Simulator only: CODE_SIGNING_ALLOWED=NO, no retries.
set -euo pipefail

python3 - <<'PY'
import json, os, subprocess, sys

def load(kind):
    return json.loads(subprocess.check_output(["xcrun", "simctl", "list", kind, "-j"]))

runtimes = []
for runtime in load("runtimes")["runtimes"]:
    if not runtime.get("isAvailable"):
        continue
    if runtime.get("platform", "iOS") != "iOS":
        continue
    name = str(runtime.get("name", ""))
    if not name.startswith("iOS"):
        continue
    parts = [int(piece) for piece in str(runtime.get("version", "0")).split(".") if piece.isdigit()]
    runtimes.append((tuple(parts), runtime["identifier"], name))
if not runtimes:
    sys.exit("no available iOS simulator runtime")
runtimes.sort()
_, runtime_id, runtime_name = runtimes[-1]

prefer = (
    "iPhone-18-Pro-Max",
    "iPhone-17-Pro-Max",
    "iPhone-16-Pro-Max",
    "iPhone-18-Pro",
    "iPhone-17-Pro",
    "iPhone-16-Pro",
)
types = {item["identifier"]: item["name"] for item in load("devicetypes")["devicetypes"]}
device = None
for suffix in prefer:
    identifier = "com.apple.CoreSimulator.SimDeviceType." + suffix
    if identifier in types:
        device = identifier
        break
if device is None:
    for identifier, dev_name in types.items():
        if dev_name.startswith("iPhone"):
            device = identifier
            break
if device is None:
    sys.exit("no iPhone simulator device type")

path = os.environ["RUNNER_TEMP"] + "/sim-choice.env"
with open(path, "w", encoding="utf-8") as handle:
    handle.write("RUNTIME=%s\nDEVTYPE=%s\n" % (runtime_id, device))
print("runtime %s (%s), device %s" % (runtime_name, runtime_id, types[device]), flush=True)
PY

# shellcheck disable=SC1091
source "$RUNNER_TEMP/sim-choice.env"
rm -f "$RUNNER_TEMP/sim-choice.env"

# macOS ships bash 3.2, where `read -a` leaves the array unset on an empty string and `set -u` then
# rejects "${array[@]}". Declare both first, and only read when there is something to split.
args=()
only_items=()
skip_items=()
[[ -n "${ONLY:-}" ]] && IFS=',' read -ra only_items <<< "$ONLY"
[[ -n "${SKIP:-}" ]] && IFS=',' read -ra skip_items <<< "$SKIP"
for item in "${only_items[@]}"; do
  [[ -n "$item" ]] && args+=(-only-testing:"$item")
done
for item in "${skip_items[@]}"; do
  [[ -n "$item" ]] && args+=(-skip-testing:"$item")
done
echo "xcodebuild ${args[*]}"

UDID="$(xcrun simctl create "Reins CI" "$DEVTYPE" "$RUNTIME")"
xcrun simctl boot "$UDID"
xcrun simctl bootstatus "$UDID" -b
echo "booted $UDID"
mkdir -p "$GITHUB_WORKSPACE/derived" "$GITHUB_WORKSPACE/spm"

# The "Rust core" build phase exits before cargo when this is set and the library is already there.
export REINS_SKIP_CORE=1
xcodebuild test \
  -project "$GITHUB_WORKSPACE/ios/Reins.xcodeproj" \
  -scheme Reins \
  -destination "platform=iOS Simulator,id=${UDID}" \
  -destination-timeout 300 \
  -derivedDataPath "$GITHUB_WORKSPACE/derived" \
  -clonedSourcePackagesDirPath "$GITHUB_WORKSPACE/spm" \
  -onlyUsePackageVersionsFromResolvedFile \
  -disableAutomaticPackageResolution \
  -resultBundlePath "$GITHUB_WORKSPACE/results.xcresult" \
  -parallel-testing-enabled NO \
  "${args[@]}" \
  CODE_SIGNING_ALLOWED=NO \
  CODE_SIGNING_REQUIRED=NO \
  CODE_SIGN_IDENTITY="" \
  DEVELOPMENT_TEAM="" \
  COMPILER_INDEX_STORE_ENABLE=NO
