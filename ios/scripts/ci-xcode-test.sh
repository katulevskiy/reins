#!/bin/bash
# One iOS build or test slice. Used by the iOS jobs in .github/workflows/ios.yml.
#
#   MODE   build | test-without-building | test (default test, the local path)
#   ONLY   comma-separated -only-testing identifiers. Required for test.
#          Empty on test-without-building exits 0 before simctl (that shard has nothing).
#   SKIP   comma-separated -skip-testing identifiers (optional)
#
# build uses a generic Simulator destination so it does not create a simulator.
# test-without-building boots one and runs the xctestrun the build job uploaded.
#
# Expects DEVELOPER_DIR, GITHUB_WORKSPACE and RUNNER_TEMP. The Rust core is the one ios-core already built
# (target/ios-core/iphonesimulator, REINS_SKIP_CORE=1). Simulator only, no retries.
#
# -collect-test-diagnostics never: after a failure xcodebuild otherwise spends 600 s timing out on
# simulator diagnostics (run 38888346599, three shards). The xcresult still records the failure.
#
# Ad-hoc identity "-" (CODE_SIGNING_ALLOWED=YES, no certificate, no team, no profile). An unsigned build
# never gets its entitlements, and SealedSnapshot's data-protection keychain item then fails to be created.
# Device status and the keys lock are dropped, which is not how a local simulator build (empty REINS_TEAM,
# still ad-hoc signed) behaves.
set -euo pipefail

MODE="${MODE:-test}"
case "$MODE" in
  build|test-without-building|test) ;;
  *) echo "MODE must be build, test-without-building, or test (got $MODE)" >&2; exit 1 ;;
esac

# A mapped pull request can leave a shard with no class. Stop before creating a simulator.
if [[ "$MODE" != build && -z "${ONLY:-}" ]]; then
  if [[ "$MODE" == test-without-building ]]; then
    echo "no tests on this shard"
    exit 0
  fi
  echo "ONLY is required" >&2
  exit 1
fi

mkdir -p "$GITHUB_WORKSPACE/derived" "$GITHUB_WORKSPACE/spm" "$GITHUB_WORKSPACE/compilation-cache"
# The "Rust core" build phase exits before cargo when this is set and the library is already there.
export REINS_SKIP_CORE=1
export COMPILATION_CACHE_ENABLE_CACHING=YES
export COMPILATION_CACHE_CAS_PATH="$GITHUB_WORKSPACE/compilation-cache"

# Passed as build settings as well as the environment. The workspace path is the same on every
# GitHub-hosted runner, so the cache key does not need prefix mapping.
sign=(
  CODE_SIGNING_ALLOWED=YES
  CODE_SIGNING_REQUIRED=NO
  CODE_SIGN_IDENTITY=-
  CODE_SIGN_STYLE=Manual
  DEVELOPMENT_TEAM=
  PROVISIONING_PROFILE_SPECIFIER=
  PROVISIONING_PROFILE=
  COMPILER_INDEX_STORE_ENABLE=NO
  COMPILATION_CACHE_ENABLE_CACHING=YES
  COMPILATION_CACHE_CAS_PATH="$GITHUB_WORKSPACE/compilation-cache"
)

if [[ "$MODE" == build ]]; then
  # generic/platform does not call simctl create. The test shards boot the simulator they run on.
  xcodebuild build-for-testing \
    -project "$GITHUB_WORKSPACE/ios/Reins.xcodeproj" \
    -scheme Reins \
    -destination 'generic/platform=iOS Simulator' \
    -sdk iphonesimulator \
    -derivedDataPath "$GITHUB_WORKSPACE/derived" \
    -clonedSourcePackagesDirPath "$GITHUB_WORKSPACE/spm" \
    -onlyUsePackageVersionsFromResolvedFile \
    -disableAutomaticPackageResolution \
    "${sign[@]}"
  exit 0
fi

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

# macOS ships bash 3.2. Under set -u, "${array[@]}" is an unbound variable when the array is empty,
# even after array=(). Shards with no -skip-testing never expand that list.
args=()
trim() {
  local value="$1"
  value="${value#"${value%%[![:space:]]*}"}"
  value="${value%"${value##*[![:space:]]}"}"
  printf '%s' "$value"
}
IFS=',' read -ra only_items <<< "$ONLY"
for item in "${only_items[@]}"; do
  item="$(trim "$item")"
  [[ -n "$item" ]] && args+=(-only-testing:"$item")
done
if [[ -n "${SKIP:-}" ]]; then
  IFS=',' read -ra skip_items <<< "$SKIP"
  for item in "${skip_items[@]}"; do
    item="$(trim "$item")"
    [[ -n "$item" ]] && args+=(-skip-testing:"$item")
  done
fi
if [[ ${#args[@]} -eq 0 ]]; then
  echo "ONLY is required" >&2
  exit 1
fi
echo "xcodebuild ${args[*]}"

UDID="$(xcrun simctl create "Reins CI" "$DEVTYPE" "$RUNTIME")"
xcrun simctl boot "$UDID"
xcrun simctl bootstatus "$UDID" -b
echo "booted $UDID"

if [[ "$MODE" == test-without-building ]]; then
  # find | head under pipefail dies with SIGPIPE (141). Bash 3.2 has no mapfile.
  xctestrun=""
  extra=""
  while IFS= read -r -d '' candidate; do
    if [[ -z "$xctestrun" ]]; then
      xctestrun="$candidate"
    else
      extra="$candidate"
      break
    fi
  done < <(find "$GITHUB_WORKSPACE/derived/Build/Products" -name '*.xctestrun' -print0)
  if [[ -z "$xctestrun" ]]; then
    echo "::error::no xctestrun under derived/Build/Products" >&2
    exit 1
  fi
  if [[ -n "$extra" ]]; then
    echo "::error::more than one xctestrun ($xctestrun and $extra)" >&2
    exit 1
  fi
  echo "xctestrun $xctestrun"
  xcodebuild test-without-building \
    -xctestrun "$xctestrun" \
    -destination "platform=iOS Simulator,id=${UDID}" \
    -destination-timeout 300 \
    -resultBundlePath "$GITHUB_WORKSPACE/results.xcresult" \
    -collect-test-diagnostics never \
    -parallel-testing-enabled NO \
    "${args[@]}" \
    "${sign[@]}"
  exit 0
fi

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
  -collect-test-diagnostics never \
  -parallel-testing-enabled NO \
  "${args[@]}" \
  "${sign[@]}"
