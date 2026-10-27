#!/usr/bin/env bash
# Builds may overlap CI, but public releases require successful push CI on this exact main commit.
set -euo pipefail
sha="${1:?commit SHA}"
[[ "$sha" =~ ^[0-9a-f]{40}$ ]] || { echo 'Invalid CI commit SHA' >&2; exit 1; }
# 30 minutes: a main push whose caches are cold (a new Cargo.lock) runs its slowest lane for about 15.
attempts="${REINS_CI_POLL_ATTEMPTS:-180}"
interval="${REINS_CI_POLL_SECONDS:-10}"
# A spent API quota (CI's macOS admission shares the token's hourly budget) resets within the hour: wait for the reset
# instead of giving up, up to this many times. Those waits do not count as attempts.
rate_waits_left="${REINS_CI_RATE_WAITS:-3}"
attempt=1
while ((attempt <= attempts)); do
    # An API error is asked again, like a run still in progress: only a successful run lets the release out.
    state="$(gh run list --workflow ci.yml --commit "$sha" --branch main --event push --limit 1 \
        --json status,conclusion --jq '.[0] | if . == null then "missing" elif .status != "completed" then .status else .conclusion end')" ||
        state=unavailable
    if [[ "$state" == unavailable ]] && ((rate_waits_left > 0)) &&
        limit="$(gh api rate_limit --jq '.resources.core | "\(.remaining) \(.reset)"' 2>/dev/null)" &&
        [[ "${limit%% *}" == 0 ]]; then
        wait=$((${limit##* } - $(date +%s) + 5))
        ((wait < 0)) && wait=0
        ((wait > 3900)) && wait=3900
        echo "The API rate limit is spent; waiting ${wait}s for it to reset"
        rate_waits_left=$((rate_waits_left - 1))
        sleep "$wait"
        continue
    fi
    case "$state" in
    success) echo "CI passed on $sha"; exit 0 ;;
    missing | queued | in_progress | pending | waiting | requested | unavailable)
        echo "Waiting for CI on $sha: $state"
        ((attempt == attempts)) || sleep "$interval"
        ;;
    *) echo "CI did not pass on $sha: ${state:-unknown status}" >&2; exit 1 ;;
    esac
    attempt=$((attempt + 1))
done
echo "CI has not completed on $sha; refusing to publish" >&2
exit 1
