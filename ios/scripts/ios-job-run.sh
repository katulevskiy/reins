#!/bin/bash
# One iOS job on a Mac (a GitHub macOS runner or the user's Mac): the prebuilt core, one build-for-testing, one
# booted simulator, then test-without-building once per requested slice. Started by .github/workflows/ios-job.yml
# and by scripts/ios-job.sh --mac; see docs/ios-dev.md.
#
#   ios/scripts/ios-job-run.sh --out DIR [--run SPECS] [--device NAME] [--derived DIR]
#
# SPECS, comma-separated (default: snapshots):
#   build                    build only (also implied by every other spec)
#   snapshots[:Screen]       ReinsTests/ScreenSnapshotTests[/testScreen]: PNGs in DIR/snapshots
#   unit[:Class[/method]]    ReinsTests, or one class or method of it
#   ui:Class[/method]        one ReinsUITests class or method (the whole UI suite runs in CI, not here)
#
# DIR gets summary.txt, summary.json, timings.tsv, logs/, results/*.xcresult (+ .json summaries) and snapshots/.
# The exit status is 0 only when the build and every slice passed.
#
# Debug, simulator only, ad-hoc signed (as ci-xcode-test.sh). Swift compilation caching is on, its store inside the
# DerivedData folder, which stays between runs: a fixed folder per checkout here, an Actions cache on GitHub. One
# simulator named "Reins Job" is created once and reused, booted while the app builds. One job at a time per Mac
# (a lock in ~/Library/Caches/reins-ios-job.lock); a second waits.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
RUN=snapshots OUT="" DEVICE="${REINS_JOB_DEVICE:-iPhone 17 Pro}" DERIVED=""
while [[ $# -gt 0 ]]; do
  case "$1" in
    --run) RUN="$2"; shift 2 ;;
    --out) OUT="$2"; shift 2 ;;
    --device) DEVICE="$2"; shift 2 ;;
    --derived) DERIVED="$2"; shift 2 ;;
    -h|--help) sed -n '2,22p' "$0"; exit 0 ;;
    *) echo "unknown argument $1" >&2; exit 2 ;;
  esac
done
[[ -n "$OUT" ]] || { echo "--out is required" >&2; exit 2; }
[[ -n "$RUN" ]] || RUN=build
mkdir -p "$OUT"
OUT="$(cd "$OUT" && pwd)"
if [[ -z "$DERIVED" ]]; then
  # One folder per checkout, so two worktrees never share build state and each keeps its own between jobs.
  key="$(printf '%s' "$ROOT" | shasum -a 256 | cut -c1-10)"
  DERIVED="$HOME/Library/Developer/Xcode/DerivedData/Reins-job-$(basename "$ROOT")-$key"
fi
mkdir -p "$DERIVED" "$OUT/logs" "$OUT/results" "$OUT/snapshots"
: > "$OUT/timings.tsv"
: > "$OUT/steps.tsv"

# --- one job per Mac -------------------------------------------------------------------------------------------
# REINS_JOB_LOCK_HELD=1: the caller (scripts/ios-job.sh --mac) already holds it, around its checkout.
LOCK="$HOME/Library/Caches/reins-ios-job.lock"
mkdir -p "$(dirname "$LOCK")"
OWN_LOCK=true
[[ "${REINS_JOB_LOCK_HELD:-}" == 1 ]] && OWN_LOCK=false
while $OWN_LOCK && ! mkdir "$LOCK" 2>/dev/null; do
  holder="$(cat "$LOCK/pid" 2>/dev/null || true)"
  if [[ -n "$holder" ]] && ! kill -0 "$holder" 2>/dev/null; then
    echo "removing a stale job lock (pid $holder)"
    rm -rf "$LOCK"
    continue
  fi
  echo "another iOS job runs on this Mac (pid ${holder:-?}); waiting"
  sleep 10
done
$OWN_LOCK && echo $$ > "$LOCK/pid"
SIM_PID=""
cleanup() {
  [[ -n "$SIM_PID" ]] && kill "$SIM_PID" 2>/dev/null || true
  if $OWN_LOCK; then rm -rf "$LOCK"; fi
}
trap cleanup EXIT

now() { date +%s; }
record() { printf '%s\t%s\t%s\n' "$1" "$2" "$3" >> "$OUT/steps.tsv"; printf '%s\t%s\n' "$1" "$2" >> "$OUT/timings.tsv"; }

# --- the requested slices --------------------------------------------------------------------------------------
# Each line: name<TAB>only-testing id<TAB>snapshots (1 or 0)
SLICES="$OUT/.slices"
: > "$SLICES"
IFS=',' read -ra specs <<< "$RUN"
for raw in "${specs[@]}"; do
  spec="$(echo "$raw" | tr -d '[:space:]')"
  [[ -z "$spec" ]] && continue
  kind="${spec%%:*}"
  arg=""
  [[ "$spec" == *:* ]] && arg="${spec#*:}"
  case "$kind" in
    build) ;;
    snapshots|snapshot)
      if [[ -n "$arg" ]]; then
        method="test$(echo "${arg:0:1}" | tr '[:lower:]' '[:upper:]')${arg:1}"
        printf 'snapshots-%s\tReinsTests/ScreenSnapshotTests/%s\t1\n' "$arg" "$method" >> "$SLICES"
      else
        printf 'snapshots\tReinsTests/ScreenSnapshotTests\t1\n' >> "$SLICES"
      fi
      ;;
    unit)
      if [[ -n "$arg" ]]; then
        printf 'unit-%s\tReinsTests/%s\t0\n' "${arg//\//-}" "$arg" >> "$SLICES"
      else
        printf 'unit\tReinsTests\t0\n' >> "$SLICES"
      fi
      ;;
    ui)
      [[ -n "$arg" ]] || { echo "ui needs a class: ui:ActivityFlowTests (the whole UI suite runs in CI)" >&2; exit 2; }
      printf 'ui-%s\tReinsUITests/%s\t0\n' "${arg//\//-}" "$arg" >> "$SLICES"
      ;;
    *) echo "unknown spec '$spec' (build, snapshots[:Screen], unit[:Class], ui:Class)" >&2; exit 2 ;;
  esac
done
TESTS=0
[[ -s "$SLICES" ]] && TESTS=1

xcodebuild -version | tr '\n' ' '; echo
echo "run=$RUN device=$DEVICE derived=$DERIVED"

# --- simulator: one, booted in the background while the app builds ----------------------------------------------
UDID_FILE="$OUT/.udid"
boot_simulator() {
  python3 - "$DEVICE" > "$UDID_FILE" <<'PY'
import json, subprocess, sys

want = sys.argv[1]
NAME = "Reins Job"

def simctl(*args):
    return subprocess.check_output(["xcrun", "simctl", *args], text=True)

def load(kind):
    return json.loads(simctl("list", kind, "-j"))

types = {t["identifier"]: t["name"] for t in load("devicetypes")["devicetypes"]}
runtimes = []
for r in load("runtimes")["runtimes"]:
    if r.get("isAvailable") and r.get("platform", "iOS") == "iOS" and str(r.get("name", "")).startswith("iOS"):
        version = tuple(int(p) for p in str(r.get("version", "0")).split(".") if p.isdigit())
        supported = [d.get("identifier") for d in r.get("supportedDeviceTypes", [])] or list(types)
        runtimes.append((version, r["identifier"], supported))
runtimes.sort(reverse=True)
if not runtimes:
    sys.exit("no available iOS simulator runtime")

preferred = [want, "iPhone 17 Pro", "iPhone 16 Pro", "iPhone 18 Pro", "iPhone 17", "iPhone 16e"]
choice = None
for name in preferred:
    ids = [i for i, n in types.items() if n == name]
    for _, rid, supported in runtimes:
        hit = [i for i in ids if i in supported]
        if hit:
            choice = (rid, hit[0])
            break
    if choice:
        break
if choice is None:
    for _, rid, supported in runtimes:
        phones = [i for i in supported if types.get(i, "").startswith("iPhone") and "Duo" not in types.get(i, "")]
        if phones:
            choice = (rid, phones[0])
            break
if choice is None:
    sys.exit("no iPhone simulator")
runtime, devtype = choice

# Reuse "Reins Job" when it is this device on this runtime; otherwise replace it.
udid = None
for rid, devices in load("devices")["devices"].items():
    for d in devices:
        if d.get("name") != NAME:
            continue
        if rid == runtime and d.get("deviceTypeIdentifier") == devtype and d.get("isAvailable", True):
            udid = d["udid"]
        else:
            subprocess.run(["xcrun", "simctl", "delete", d["udid"]], check=False)
if udid is None:
    udid = simctl("create", NAME, devtype, runtime).strip()
sys.stderr.write(f"simulator {NAME}: {types[devtype]}, {runtime}, {udid}\n")
print(udid)
PY
  local udid
  udid="$(cat "$UDID_FILE")"
  xcrun simctl boot "$udid" 2>/dev/null || true
  xcrun simctl bootstatus "$udid" -b >/dev/null
}

if [[ "$TESTS" == 1 ]]; then
  t0=$(now)
  ( boot_simulator > "$OUT/logs/simulator.log" 2>&1; echo "$(( $(now) - t0 ))" > "$OUT/.boot-seconds" ) &
  SIM_PID=$!
fi

# --- the Rust core: prebuilt for this hash, else built once and stamped -------------------------------------------
t0=$(now)
status=pass
if ! "$ROOT/ios/scripts/core-cache.sh" ensure iphonesimulator 2>&1 | tee "$OUT/logs/core.log"; then status=fail; fi
record core "$(( $(now) - t0 ))" "$status"
if [[ "$status" == fail ]]; then
  echo "the Rust core is not available" >&2
  exit 1
fi

# --- build once ----------------------------------------------------------------------------------------------------
settings=(
  CODE_SIGNING_ALLOWED=YES
  CODE_SIGNING_REQUIRED=NO
  CODE_SIGN_IDENTITY=-
  CODE_SIGN_STYLE=Manual
  DEVELOPMENT_TEAM=
  PROVISIONING_PROFILE_SPECIFIER=
  PROVISIONING_PROFILE=
  COMPILER_INDEX_STORE_ENABLE=NO
  COMPILATION_CACHE_ENABLE_CACHING=YES
  ONLY_ACTIVE_ARCH=YES
  DEBUG_INFORMATION_FORMAT=dwarf
)
export REINS_SKIP_CORE=1
t0=$(now)
status=pass
# generic/platform: no simulator needed to build, so the boot above runs alongside.
if ! xcodebuild build-for-testing \
  -project "$ROOT/ios/Reins.xcodeproj" \
  -scheme Reins \
  -configuration Debug \
  -destination 'generic/platform=iOS Simulator' \
  -derivedDataPath "$DERIVED" \
  -clonedSourcePackagesDirPath "$DERIVED/SourcePackages" \
  -onlyUsePackageVersionsFromResolvedFile \
  -skipPackagePluginValidation \
  "${settings[@]}" > "$OUT/logs/build.log" 2>&1; then
  status=fail
fi
record build "$(( $(now) - t0 ))" "$status"
grep -E "^(/.*: )?(error|fatal error):|\*\* BUILD" "$OUT/logs/build.log" | sort -u | head -60 || true

finish() {
  python3 - "$OUT" "$RUN" <<'PY'
import json, os, sys
out, run = sys.argv[1], sys.argv[2]
steps = []
with open(os.path.join(out, "steps.tsv"), encoding="utf-8") as f:
    for line in f:
        name, seconds, status = line.rstrip("\n").split("\t")
        step = {"step": name, "seconds": int(seconds), "status": status}
        result = os.path.join(out, "results", name + ".json")
        if os.path.exists(result):
            try:
                with open(result, encoding="utf-8") as r:
                    data = json.load(r)
                step["tests"] = {k: data.get(k) for k in ("totalTestCount", "passedTests", "failedTests", "skippedTests")}
                step["failures"] = [
                    {"test": x.get("testIdentifierString") or x.get("testName"), "text": x.get("failureText", "")}
                    for x in data.get("testFailures", [])
                ]
            except (OSError, ValueError):
                pass
        steps.append(step)
snapshots = sorted(p for p in os.listdir(os.path.join(out, "snapshots")) if p.endswith(".png"))
ok = all(s["status"] == "pass" for s in steps)
with open(os.path.join(out, "summary.json"), "w", encoding="utf-8") as f:
    json.dump({"ok": ok, "run": run, "steps": steps, "snapshots": snapshots}, f, indent=2)
lines = [f"{'PASS' if ok else 'FAIL'}  {run}"]
for s in steps:
    extra = ""
    if "tests" in s:
        t = s["tests"]
        extra = f"  {t.get('passedTests')} passed, {t.get('failedTests')} failed, {t.get('skippedTests')} skipped"
    lines.append(f"  {s['status']:<4}  {s['step']:<32} {s['seconds']:>5} s{extra}")
    for x in s.get("failures", [])[:20]:
        lines.append(f"        {x['test']}: {x['text']}")
if snapshots:
    lines.append(f"  {len(snapshots)} snapshots in snapshots/")
text = "\n".join(lines) + "\n"
with open(os.path.join(out, "summary.txt"), "w", encoding="utf-8") as f:
    f.write(text)
print(text, end="")
sys.exit(0 if ok else 1)
PY
}

if [[ "$status" == fail ]]; then
  echo "build failed: $OUT/logs/build.log" >&2
  finish || true
  exit 1
fi
[[ "$TESTS" == 1 ]] || { finish; exit $?; }

# --- the simulator, then each slice without building --------------------------------------------------------------
t0=$(now)
if ! wait "$SIM_PID"; then
  cat "$OUT/logs/simulator.log" >&2
  record simulator "$(( $(now) - t0 ))" fail
  finish || true
  exit 1
fi
SIM_PID=""
UDID="$(cat "$UDID_FILE")"
record simulator "$(cat "$OUT/.boot-seconds" 2>/dev/null || echo 0)" pass
cat "$OUT/logs/simulator.log"

xctestrun=""
while IFS= read -r -d '' candidate; do
  [[ "$candidate" == */job-*.xctestrun ]] && continue
  xctestrun="$candidate"
done < <(find "$DERIVED/Build/Products" -maxdepth 1 -name '*.xctestrun' -print0)
[[ -n "$xctestrun" ]] || { echo "no xctestrun under $DERIVED/Build/Products" >&2; exit 1; }

# The test process sees only the environment the xctestrun gives it: CI (as in CI, when this runs there) and, for
# snapshot slices, REINS_SNAPSHOTS.
prepare_xctestrun() {
  python3 - "$xctestrun" "$1" "$2" <<'PY'
import os, plistlib, sys
source, target, snapshots = sys.argv[1], sys.argv[2], sys.argv[3]
with open(source, "rb") as f:
    data = plistlib.load(f)
env = {}
if os.environ.get("CI"):
    env["CI"] = "1"
if snapshots:
    env["REINS_SNAPSHOTS"] = snapshots
def apply(target_dict):
    for key in ("EnvironmentVariables", "TestingEnvironmentVariables"):
        target_dict.setdefault(key, {}).update(env)
if "TestConfigurations" in data:  # format version 2
    for config in data["TestConfigurations"]:
        for t in config.get("TestTargets", []):
            apply(t)
else:
    for key, value in data.items():
        if not key.startswith("__") and isinstance(value, dict):
            apply(value)
with open(target, "wb") as f:
    plistlib.dump(data, f, fmt=plistlib.FMT_XML)
PY
}

overall=0
while IFS=$'\t' read -r name only snaps; do
  [[ -z "$name" ]] && continue
  run_file="$(dirname "$xctestrun")/job-$name.xctestrun"
  snap_dir=""
  [[ "$snaps" == 1 ]] && snap_dir="$OUT/snapshots"
  prepare_xctestrun "$run_file" "$snap_dir"
  bundle="$OUT/results/$name.xcresult"
  rm -rf "$bundle"
  echo "=== $name: -only-testing:$only"
  t0=$(now)
  status=pass
  if ! xcodebuild test-without-building \
    -xctestrun "$run_file" \
    -destination "platform=iOS Simulator,id=$UDID" \
    -destination-timeout 300 \
    -resultBundlePath "$bundle" \
    -collect-test-diagnostics never \
    -parallel-testing-enabled NO \
    -only-testing:"$only" < /dev/null > "$OUT/logs/$name.log" 2>&1; then
    status=fail
    overall=1
  fi
  record "$name" "$(( $(now) - t0 ))" "$status"
  rm -f "$run_file"
  if [[ -d "$bundle" ]]; then
    xcrun xcresulttool get test-results summary --path "$bundle" < /dev/null > "$OUT/results/$name.json" 2>/dev/null || true
  fi
  grep -E "Test Case .* (passed|failed)|error:|\*\* TEST" "$OUT/logs/$name.log" | tail -40 || true
done < "$SLICES"
rm -f "$SLICES" "$UDID_FILE" "$OUT/.boot-seconds"

finish
