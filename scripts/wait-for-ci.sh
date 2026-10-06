#!/usr/bin/env bash
# Builds may overlap CI, but public releases require successful push CI on this exact main commit.
set -euo pipefail
sha="${1:?commit SHA}"
[[ "$sha" =~ ^[0-9a-f]{40}$ ]] || { echo 'Invalid CI commit SHA' >&2; exit 1; }
attempts="${REINS_CI_POLL_ATTEMPTS:-90}"
interval="${REINS_CI_POLL_SECONDS:-10}"
for ((attempt = 1; attempt <= attempts; attempt++)); do
    state="$(gh run list --workflow ci.yml --commit "$sha" --branch main --event push --limit 1 \
        --json status,conclusion --jq '.[0] | if . == null then "missing" elif .status != "completed" then .status else .conclusion end')"
    case "$state" in
    success) echo "CI passed on $sha"; exit 0 ;;
    missing | queued | in_progress | pending | waiting | requested)
        echo "Waiting for CI on $sha: $state"
        ((attempt == attempts)) || sleep "$interval"
        ;;
    *) echo "CI did not pass on $sha: ${state:-unknown status}" >&2; exit 1 ;;
    esac
done
echo "CI has not completed on $sha; refusing to publish" >&2
exit 1
