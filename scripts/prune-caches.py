#!/usr/bin/env python3
"""Deletes the GitHub Actions caches nothing restores any more, so the ones main's runs restore stay under GitHub's
10 GB per repository. Past that GitHub evicts the least recently used caches first, and those are the ones a release
restores once per run: the macOS build's Rust cache went within minutes of its save (release 40476007982).

Three kinds go (.github/workflows/cache-prune.yml runs this after CI and iOS on main, and every hour):
  - superseded: every key ends in hashes of its inputs (Cargo.lock, Package.resolved, the sources). Of one family (the
    key without its trailing hashes) only the most recently used entry is restored again: rust-cache and the Swift
    cache fall back to the newest entry of the family, the exact-source caches match only the current hash. The others
    go once they have gone unused for GRACE (a run that is restoring one now finishes with it).
  - another ref's: a pull request (or another branch) restores main's caches and writes none of its own (save-if main);
    one that a branch still wrote goes once unused for BRANCH_GRACE.
  - retired: a key no workflow writes or restores any more (RETIRED), once unused for GRACE.

  scripts/prune-caches.py [--dry-run]

Environment: GH_REPO (owner/name), GH_TOKEN with actions: write (read for --dry-run). Writes a summary to
GITHUB_STEP_SUMMARY when set. scripts/test-prune-caches.py tests the plan.
"""

import datetime as dt
import json
import os
import re
import subprocess
import sys

MAIN = "refs/heads/main"
GRACE = dt.timedelta(minutes=10)
BRANCH_GRACE = dt.timedelta(hours=1)
LIMIT = 10 * 1000**3
# Warn when main's own caches (after pruning) leave less than this for a run's new entries before eviction starts.
HEADROOM = 1 * 1000**3
HASH = re.compile(r"-[0-9a-f]{8,}$")
# Keys the workflows no longer use (10-27: the release's Linux, Windows, server, Android and iOS core Rust caches,
# ios-core.yml's own, and the exact-source keys that hashed a workflow file). Kept here so their last copies go at once
# instead of sitting in the 10 GB for the seven days GitHub keeps an unused cache.
RETIRED = (
    "v0-rust-release-server-",
    "v0-rust-release-desktop-app-Linux-",
    "v0-rust-release-desktop-app-Windows-",
    "v0-rust-release-android-native-",
    "v0-rust-release-ios-core-",
    "v0-rust-macos-ios-core-",
    "ios-core-v1-",
    "android-native-v1-",
    "android-native-dev-v1-",
)


def family(key):
    """The key without its trailing input hashes: v0-rust-<name>-<os>-<arch>-<env hash>-<lock hash> -> v0-rust-<name>-
    <os>-<arch>; ios-swift-v1-macOS-<hash> -> ios-swift-v1-macOS. A key with no hash is its own family."""
    while True:
        shorter = HASH.sub("", key)
        if shorter == key:
            return key
        key = shorter


def when(text):
    return dt.datetime.fromisoformat(text.replace("Z", "+00:00"))


def plan(caches, now):
    """Returns (delete, keep): lists of (cache, reason) and caches."""
    delete, keep = [], []
    families = {}
    for cache in caches:
        if cache["ref"] != MAIN:
            if now - when(cache["last_accessed_at"]) > BRANCH_GRACE:
                delete.append((cache, f"{cache['ref']}, unused for an hour"))
            else:
                keep.append(cache)
            continue
        if cache["key"].startswith(RETIRED):
            if now - when(cache["last_accessed_at"]) > GRACE:
                delete.append((cache, "retired: no workflow uses this key"))
            else:
                keep.append(cache)
            continue
        families.setdefault(family(cache["key"]), []).append(cache)
    for name, members in families.items():
        members.sort(key=lambda c: (when(c["last_accessed_at"]), when(c["created_at"]), c["id"]), reverse=True)
        keep.append(members[0])
        for cache in members[1:]:
            if now - when(cache["last_accessed_at"]) > GRACE:
                delete.append((cache, f"superseded by {members[0]['key']}"))
            else:
                keep.append(cache)
    return delete, keep


def gh(*args):
    return subprocess.run(["gh", *args], check=True, capture_output=True, text=True).stdout


def size(caches):
    return sum(c["size_in_bytes"] for c in caches)


def gb(n):
    return f"{n / 1000**3:.2f} GB"


def main():
    dry = "--dry-run" in sys.argv[1:]
    repo = os.environ["GH_REPO"]
    out = gh("api", "--paginate", f"repos/{repo}/actions/caches?per_page=100", "--jq", ".actions_caches[]")
    caches = [json.loads(line) for line in out.splitlines() if line.strip()]
    now = dt.datetime.now(dt.timezone.utc)
    delete, keep = plan(caches, now)
    lines = [f"Caches: {len(caches)}, {gb(size(caches))} of {gb(LIMIT)}", ""]
    failed = 0
    for cache, reason in sorted(delete, key=lambda d: -d[0]["size_in_bytes"]):
        verb = "would delete" if dry else "deleted"
        if not dry:
            try:
                gh("api", "-X", "DELETE", f"repos/{repo}/actions/caches/{cache['id']}")
            except subprocess.CalledProcessError as error:
                # Already gone (evicted, or another run deleted it) is fine; anything else is reported.
                if "404" not in (error.stderr or ""):
                    failed += 1
                    print(f"::warning::could not delete {cache['key']}: {(error.stderr or '').strip()}")
                    keep.append(cache)
                    continue
        lines.append(f"- {verb} {cache['key']} ({gb(cache['size_in_bytes'])}): {reason}")
    lines += ["", f"Kept: {len(keep)}, {gb(size(keep))}", ""]
    for cache in sorted(keep, key=lambda c: -c["size_in_bytes"]):
        lines.append(f"- {cache['key']} ({gb(cache['size_in_bytes'])}, {cache['ref']}, "
                     f"used {cache['last_accessed_at']})")
    text = "\n".join(lines)
    print(text)
    summary = os.environ.get("GITHUB_STEP_SUMMARY")
    if summary:
        with open(summary, "a", encoding="utf-8") as handle:
            handle.write("## Actions caches\n\n" + text + "\n")
    if size(keep) > LIMIT - HEADROOM:
        print(f"::warning::The caches kept take {gb(size(keep))} of {gb(LIMIT)}: a run that writes new ones "
              "will make GitHub evict the least recently used (a release's first)")
    return 1 if failed else 0


if __name__ == "__main__":
    sys.exit(main())
