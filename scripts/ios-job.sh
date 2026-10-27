#!/usr/bin/env bash
# Runs an iOS build/test job for the current branch somewhere with Xcode, waits, and copies the results here.
#
#   scripts/ios-job.sh [SPECS] [--device NAME] [--mac] [--no-wait] [--watch RUN_ID]
#
# SPECS, comma-separated (default: snapshots): build | snapshots[:Screen] | unit[:Class[/method]] | ui:Class[/method].
#   scripts/ios-job.sh snapshots                      every screen, light and dark, as PNGs
#   scripts/ios-job.sh snapshots:Approve              one screen (ScreenSnapshotTests/testApprove)
#   scripts/ios-job.sh unit:GrantLogicTests           one unit-test class
#   scripts/ios-job.sh build,unit:GrantLogicTests,snapshots:Trust   one build, then both
#
# Default: GitHub's macOS runners (.github/workflows/ios-job.yml) build the branch's pushed head, so push first; the
# job runs that exact commit. --mac runs it on the user's Mac over SSH instead (REINS_MAC_SSH=user@host, the checkout
# REINS_MAC_REPO, default ~/reins-jobs; one job at a time there; see docs/ios-dev.md for the one-time setup).
# --watch RUN_ID waits for and downloads a run already started. --no-wait prints the run and returns.
#
# Results land in ~/Documents/dev/tmp/ios-jobs/<run>/ (REINS_IOS_JOBS overrides): summary.txt, summary.json,
# snapshots/*.png, logs/, results/. Exit status 0 only when the job passed.
set -euo pipefail

REPO="${GH_REPO:-katulevskiy/reins}"
export GH_REPO="$REPO"
JOBS="${REINS_IOS_JOBS:-$HOME/Documents/dev/tmp/ios-jobs}"
RUN="" DEVICE="iPhone 17 Pro" MAC=false WAIT=true WATCH=""
while [[ $# -gt 0 ]]; do
  case "$1" in
    --device) DEVICE="$2"; shift 2 ;;
    --mac) MAC=true; shift ;;
    --no-wait) WAIT=false; shift ;;
    --watch) WATCH="$2"; shift 2 ;;
    -h|--help) sed -n '2,22p' "$0"; exit 0 ;;
    -*) echo "unknown option $1" >&2; exit 2 ;;
    *) RUN="${RUN:+$RUN,}$1"; shift ;;
  esac
done
RUN="${RUN:-snapshots}"
START=$(date +%s)
elapsed() { echo "$(( $(date +%s) - START )) s"; }

report() {
  local dir="$1"
  echo
  if [[ -f "$dir/summary.txt" ]]; then cat "$dir/summary.txt"; else echo "no summary.txt in $dir"; fi
  if compgen -G "$dir/snapshots/*.png" >/dev/null; then
    echo "snapshots:"
    ls -1 "$dir"/snapshots/*.png | sed 's/^/  /'
  fi
  echo "results: $dir"
  echo "total: $(elapsed)"
  python3 -c 'import json,sys; sys.exit(0 if json.load(open(sys.argv[1])).get("ok") else 1)' "$dir/summary.json" 2>/dev/null
}

branch="$(git rev-parse --abbrev-ref HEAD)"
sha="$(git rev-parse HEAD)"

# --- the Mac over SSH ----------------------------------------------------------------------------------------------
if $MAC; then
  host="${REINS_MAC_SSH:?set REINS_MAC_SSH=user@host (docs/ios-dev.md, Mac setup)}"
  remote_repo="${REINS_MAC_REPO:-reins-jobs}"
  [[ "$branch" != HEAD ]] || { echo "detached HEAD: check out a branch and push it" >&2; exit 1; }
  remote_head="$(gh api "repos/$REPO/branches/$branch" --jq .commit.sha 2>/dev/null || true)"
  [[ "$remote_head" == "$sha" ]] || { echo "push $branch first: the Mac fetches $sha from GitHub (remote has ${remote_head:-nothing})" >&2; exit 1; }
  id="mac-$(date +%Y%m%d-%H%M%S)"
  out="$JOBS/$id"
  mkdir -p "$out"
  echo "running '$RUN' for $branch@${sha:0:10} on $host"
  remote_out="Library/Caches/reins-ios-jobs/$id"
  status=0
  # The lock covers the checkout too, so a second job never switches the tree under a build.
  ssh -o BatchMode=yes "$host" "bash -l -s -- $(printf '%q ' "$remote_repo" "$branch" "$sha" "$RUN" "$DEVICE" "$remote_out" "${REINS_MAC_DEVELOPER_DIR:-}")" <<'REMOTE' || status=$?
set -euo pipefail
repo="$1" branch="$2" sha="$3" run="$4" device="$5" out="$6" developer="$7"
[[ -n "$developer" ]] && export DEVELOPER_DIR="$developer"
cd "$HOME"
lock="$HOME/Library/Caches/reins-ios-job.lock"
mkdir -p "$HOME/Library/Caches"
while ! mkdir "$lock" 2>/dev/null; do
  pid="$(cat "$lock/pid" 2>/dev/null || true)"
  if [[ -n "$pid" ]] && ! kill -0 "$pid" 2>/dev/null; then rm -rf "$lock"; continue; fi
  echo "another iOS job runs on this Mac (pid ${pid:-?}); waiting"
  sleep 10
done
echo $$ > "$lock/pid"
trap 'rm -rf "$lock"' EXIT
cd "$repo"
git fetch -q origin "$branch"
git checkout -q --detach "$sha"
REINS_JOB_LOCK_HELD=1 ios/scripts/ios-job-run.sh --out "$HOME/$out" --run "$run" --device "$device"
REMOTE
  scp -q -r "$host:$remote_out/." "$out/" || echo "could not copy the results back" >&2
  ssh -o BatchMode=yes "$host" "rm -rf $(printf '%q' "$remote_out")" || true
  report "$out" || status=1
  exit "$status"
fi

# --- GitHub's macOS runners ----------------------------------------------------------------------------------------
if [[ -z "$WATCH" ]]; then
  [[ "$branch" != HEAD ]] || { echo "detached HEAD: check out a branch and push it" >&2; exit 1; }
  remote_head="$(gh api "repos/$REPO/branches/$branch" --jq .commit.sha 2>/dev/null || true)"
  if [[ "$remote_head" != "$sha" ]]; then
    echo "push $branch first: the job builds what GitHub has (${remote_head:-no such branch}), not $sha" >&2
    exit 1
  fi
  tag="j$(date +%s)$RANDOM"
  echo "dispatching '$RUN' for $branch@${sha:0:10} ($tag)"
  gh workflow run ios-job.yml --ref "$branch" -f ref="$sha" -f run="$RUN" -f device="$DEVICE" -f id="$tag"
  for _ in $(seq 1 30); do
    WATCH="$(gh run list --workflow ios-job.yml --branch "$branch" --event workflow_dispatch --limit 20 \
      --json databaseId,displayTitle --jq ".[] | select(.displayTitle | endswith(\"$tag\")) | .databaseId" | head -1)"
    [[ -n "$WATCH" ]] && break
    sleep 2
  done
  [[ -n "$WATCH" ]] || { echo "the run did not appear; see gh run list --workflow ios-job.yml" >&2; exit 1; }
fi
url="https://github.com/$REPO/actions/runs/$WATCH"
echo "run $WATCH: $url"
$WAIT || exit 0

last=""
while true; do
  state="$(gh run view "$WATCH" --json status,conclusion --jq '"\(.status) \(.conclusion)"' 2>/dev/null || echo "unknown ")"
  if [[ "$state" != "$last" ]]; then echo "$(elapsed)  $state"; last="$state"; fi
  [[ "$state" == completed* ]] && break
  sleep 15
done
conclusion="${state#completed }"

out="$JOBS/$WATCH"
rm -rf "$out"
mkdir -p "$out"
if ! gh run download "$WATCH" -n ios-job -D "$out" 2>/dev/null; then
  echo "no ios-job artifact; the job failed before it ran (see $url)" >&2
  gh run view "$WATCH" --log-failed 2>/dev/null | tail -40 >&2 || true
  exit 1
fi
status=0
report "$out" || status=1
[[ "$conclusion" == success ]] || status=1
echo "run: $url ($conclusion)"
exit "$status"
