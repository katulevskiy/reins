#!/usr/bin/env python3
"""The cache prune deletes only what nothing restores again (a family's older entries, another ref's idle ones), and
the workflows keep the repository's caches inside GitHub's 10 GB: one key per job, written from main only."""

import datetime as dt
import importlib.util
import re
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
spec = importlib.util.spec_from_file_location("prune", ROOT / "scripts/prune-caches.py")
prune = importlib.util.module_from_spec(spec)
spec.loader.exec_module(prune)
checks = 0


def check(condition, message):
    global checks
    checks += 1
    assert condition, message


NOW = dt.datetime(2026, 10, 27, 12, 0, tzinfo=dt.timezone.utc)
ids = iter(range(1, 1000))


def cache(key, used_minutes_ago, ref="refs/heads/main", created_minutes_ago=None, size=100):
    def stamp(minutes):
        return (NOW - dt.timedelta(minutes=minutes)).isoformat().replace("+00:00", "Z")
    return {"id": next(ids), "key": key, "ref": ref, "size_in_bytes": size,
            "last_accessed_at": stamp(used_minutes_ago),
            "created_at": stamp(created_minutes_ago if created_minutes_ago is not None else used_minutes_ago)}


# Families: the key without its trailing input hashes.
for key, expected in (
    ("v0-rust-linux-server-core-Linux-x64-04836758-a26f7ae0", "v0-rust-linux-server-core-Linux-x64"),
    ("v0-rust-desktop-windows-Windows_NT-x64-57172357-3cd8a9e7", "v0-rust-desktop-windows-Windows_NT-x64"),
    ("v0-rust-android-debug-native-arm64-v8a-Linux-x64-04836758-a26f7ae0",
     "v0-rust-android-debug-native-arm64-v8a-Linux-x64"),
    ("ios-swift-v1-macOS-" + "ab" * 32, "ios-swift-v1-macOS"),
    ("android-native-v2-Linux-x86_64-" + "cd" * 32, "android-native-v2-Linux-x86_64"),
    ("android-native-v2-Linux-bindings-" + "cd" * 32, "android-native-v2-Linux-bindings"),
    ("robolectric-" + "ef" * 32, "robolectric"),
    ("setup-java-Linux-x64-gradle-" + "01" * 32, "setup-java-Linux-x64-gradle"),
    ("cargo-ndk-Linux-4.1.2", "cargo-ndk-Linux-4.1.2"),
):
    check(prune.family(key) == expected, (key, prune.family(key)))

old_server = cache("v0-rust-linux-server-core-Linux-x64-04836758-3cd8a9e7", 300)
new_server = cache("v0-rust-linux-server-core-Linux-x64-04836758-a26f7ae0", 30)
# A new Rust toolchain changes the environment hash: the older one is superseded too.
old_toolchain = cache("v0-rust-linux-server-core-Linux-x64-11111111-a26f7ae0", 200)
# Restored a few minutes ago by a run that may still be using it: kept for now.
busy = cache("v0-rust-linux-desktop-ui-Linux-x64-04836758-3cd8a9e7", 3)
ui = cache("v0-rust-linux-desktop-ui-Linux-x64-04836758-a26f7ae0", 1)
# A release cache used once, hours ago, alone in its family: kept.
macos = cache("v0-rust-release-desktop-app-macOS-both-Darwin-arm64-5976a727-3cd8a9e7", 600)
# Same key family, but the older was used last (a newer save from an older commit): the one in use stays.
swift_used = cache("ios-swift-v1-macOS-" + "aa" * 32, 5, created_minutes_ago=400)
swift_stale = cache("ios-swift-v1-macOS-" + "bb" * 32, 100, created_minutes_ago=100)
# Distinct variants are distinct families.
arm = cache("android-native-v2-Linux-arm64-v8a-" + "cd" * 32, 50)
x86 = cache("android-native-v2-Linux-x86_64-" + "cd" * 32, 50)
# Another ref's caches: idle ones go, one in use stays.
pr_idle = cache("v0-rust-linux-server-core-Linux-x64-04836758-ffffffff", 120, ref="refs/pull/12/merge")
pr_busy = cache("v0-rust-linux-server-core-Linux-x64-04836758-eeeeeeee", 20, ref="refs/pull/13/merge")
branch_idle = cache("ios-swift-v1-macOS-" + "cc" * 32, 90, ref="refs/heads/ci/cache-budget")
# Keys no workflow uses any more go, even alone in their family; the current ones of similar names stay.
retired = cache("v0-rust-release-server-Linux-x64-6b4eadc9-a26f7ae0", 60)
retired_ios = cache("ios-core-v1-27.0-" + "38" * 32, 30)
retired_native = cache("android-native-dev-v1-Linux-x86_64-" + "3e" * 32, 30)
everything = [old_server, new_server, old_toolchain, busy, ui, macos, swift_used, swift_stale, arm, x86, pr_idle,
              pr_busy, branch_idle, retired, retired_ios, retired_native]
delete, keep = prune.plan(everything, NOW)
deleted = {c["id"] for c, _ in delete}
check(deleted == {old_server["id"], old_toolchain["id"], swift_stale["id"], pr_idle["id"], branch_idle["id"],
                  retired["id"], retired_ios["id"], retired_native["id"]},
      sorted(c["key"] for c, _ in delete))
check(not any(prune.family(c["key"]).startswith(prune.RETIRED) for c in keep), "nothing retired kept")
check({c["id"] for c in keep} | deleted == {c["id"] for c in everything} and not deleted & {c["id"] for c in keep},
      "every cache is either kept or deleted")
reasons = {c["id"]: why for c, why in delete}
check(reasons[old_server["id"]] == f"superseded by {new_server['key']}", reasons[old_server["id"]])
check("refs/pull/12/merge" in reasons[pr_idle["id"]], reasons[pr_idle["id"]])
check(prune.plan([], NOW) == ([], []), "nothing to do")

# The workflows: caches are written from main only, and the release keeps no Rust cache a job restores once per run
# except the macOS build's, which publication waits for.
workflows = {path.name: path.read_text() for path in (ROOT / ".github/workflows").glob("*.yml")}
keys = {}
for name, text in workflows.items():
    for block in re.findall(r"uses: Swatinem/rust-cache@[^\n]*\n((?:        [^\n]*\n)+)", text):
        save = re.search(r"save-if: (.*)", block)
        check(save, f"{name}: every rust-cache says when it saves")
        value = save.group(1)
        check(value == "false" or "refs/heads/main" in value or "trusted" in value, f"{name}: saves from main: {value}")
        shared = re.search(r"shared-key: (.*)", block)
        keys.setdefault(shared.group(1) if shared else "(job)", []).append(name)
release = workflows["release.yml"]
check(len(re.findall(r"uses: Swatinem/rust-cache@", release)) == 2, "release: the macOS build's, and the bindings' read")
check("shared-key: release-desktop-app-macOS-both" in release, "the macOS build keeps its cache")
check("shared-key: linux-server-core\n          save-if: false" in release, "the bindings read CI's cache only")
for gone in ("release-server", "release-desktop-app-Linux", "release-desktop-app-Windows", "release-android-native",
             "release-ios-core", "macos-ios-core-"):
    check(gone not in "".join(workflows.values()), f"{gone} keeps no cache")
# Every retired prefix is a key no workflow writes: none of them is still in a workflow.
for prefix in prune.RETIRED:
    stem = prefix.removeprefix("v0-rust-").rstrip("-")
    check(not re.search(rf"(shared-key: |key: ){re.escape(stem)}", "".join(workflows.values())), f"{prefix} in use")
check(keys.get("macos-ios-sim-core") == ["ios-core.yml", "ios.yml"] or
      sorted(keys.get("macos-ios-sim-core", [])) == ["ios-core.yml", "ios.yml"], keys.get("macos-ios-sim-core"))
# No cache key hashes a workflow file: an edit to the workflow would orphan the cache.
for name, text in workflows.items():
    for files in re.findall(r"hashFiles\(([^)]*)\)", text):
        check(".github/" not in files, f"{name}: a workflow in a cache key: {files}")
# The TestFlight job restores no Actions cache: its core is the attested release asset or built there.
job = release.split("\n  ios-testflight:\n", 1)[1].split("\n\n  # ", 1)[0]
check("actions/cache" not in job and "rust-cache" not in job, "TestFlight restores no Actions cache")
check("core-cache.sh fetch iphoneos-release" in job, "TestFlight fetches the published Release core")
# The prune: its own job holds actions: write; CI and iOS call it on main.
prune_workflow = workflows["cache-prune.yml"]
check("permissions: {}" in prune_workflow and prune_workflow.count("actions: write") == 1, "actions: write in one job")
check("python3 scripts/prune-caches.py" in prune_workflow, "the script")
for name in ("ci.yml", "ios.yml"):
    caller = workflows[name].split("\n  cache-prune:\n", 1)[1].split("\n\n", 1)[0]
    check("uses: ./.github/workflows/cache-prune.yml" in caller and "actions: write" in caller
          and "github.ref == 'refs/heads/main'" in caller and "pull_request" in caller, f"{name} calls the prune")

print(f"{checks} cache checks passed")
