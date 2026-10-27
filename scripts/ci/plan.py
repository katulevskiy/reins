#!/usr/bin/env python3
"""Decide which CI suites a run needs, which already passed on the same inputs, and the macOS runners it asks for.

Run by the `plan` job of .github/workflows/ci.yml. Inputs come from the environment (see main()); outputs go to
$GITHUB_OUTPUT:

  suites   JSON object, suite -> whether its jobs run
  keys     JSON object, suite -> the cache key prefix its jobs record a pass under (scripts/ci/reuse.py)
  macos    the macOS jobs the run starts (desktop-macos, iOS): its request to macOS admission (scripts/ci/admit.py)
  docker   whether the checks job lints and builds the Docker templates
  tier     fast (pull requests) or full (main, manual runs, the schedule, the `ci:full` label, CI_PR_TIER=full)
  pr       the pull request number, for the admission gate's name (`ci:priority`, `ci:lock`)

Selection. A suite runs when a changed file is one of its inputs: the files its fingerprint hashes (reuse.py), so
what selects a suite and what decides whether its pass still holds are the same list. Its Rust inputs come from the
Cargo graph: a change to reins-core runs the server, Android and iOS suites, not the desktop ones, which do not depend
on it. Changes to CI itself (ci.yml, scripts/ci, the admission gate) run everything.

Fast tier (pull requests). macOS runners are the only scarce ones (five for the account, shared with releases;
docs/reference/ci.md). desktop-macos runs on a pull request only when the change can plausibly be macOS specific: the
desktop crates themselves, the lockfile, toolchain or cargo config, or changed code (not comments) in another of its
crates with a platform conditional or macOS API (target_os, cfg(unix), objc...). Main runs it for every change to its
inputs. Linux and Windows suites run on pull requests as on main.

iOS (one macOS job: the core, the build and ReinsTests) runs here for pull requests and manual runs. On main, ios.yml
runs it with the UI tests, in the background: releases wait for this workflow, and iOS never holds one.

Reuse. A selected suite whose inputs are unchanged since all its jobs passed, on this pull request or on main, is
skipped (counted as passed by `CI result`). Main reuses main's passes only; manual runs, the schedule and `ci:full`
reuse nothing. A changed Rust build file that no suite's inputs cover turns reuse off for the run (fail closed).
"""

from __future__ import annotations

import json
import os
import re
import subprocess
import sys
import urllib.request

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import reuse  # noqa: E402
from reuse import matches  # noqa: E402

# Suite -> (job, runner) for each job it starts; the job is also the name its pass is recorded under ("Record the
# pass" steps in ci.yml; scripts/test-ci-plan.py checks they match).
SUITE_JOBS: dict[str, list[tuple[str, str]]] = {
    "rust": [("rust", "linux")],
    "clippy": [("clippy", "linux")],
    "replay": [("replay", "linux")],
    "packages": [("packages", "linux")],
    "desktop-linux": [("desktop-linux", "linux")],
    "desktop-macos": [("desktop-macos", "macos")],
    "desktop-windows": [("desktop-windows", "windows")],
    "android": [("native-arm64-v8a", "linux"), ("native-x86_64", "linux"), ("native-bindings", "linux"),
                ("full", "linux"), ("play", "linux"), ("apk", "linux"), ("smoke", "linux")],
    "ios": [("ios", "macos")],
}
assert list(SUITE_JOBS) == reuse.SUITES

CI_SELF = [".github/workflows/ci.yml", ".github/workflows/macos-gate.yml", "scripts/ci/**"]
DOCKER = ["docker/**", "Dockerfile", ".dockerignore", ".hadolint.yaml"]

# Fast tier: the files that always select desktop-macos on a pull request...
FAST_PATHS: dict[str, list[str]] = {
    "desktop-macos": ["Cargo.toml", "Cargo.lock", "rust-toolchain.toml", ".cargo/**", "crates/reins-desktop/**",
                      "crates/reins-desktop-app/**"],
}
# ...and, in its other inputs under crates/, a macOS-looking file name or changed code with a platform conditional or
# a macOS API. A platform conditional counts because it decides what macOS compiles. A Rust file without a diff (too
# large, binary) counts too.
PLATFORM_CFG = r"target_os|target_family|target_vendor|cfg!?\(\s*(?:not\(\s*)?(?:unix|windows)\b"
PLATFORM_PATHS = {"desktop-macos": re.compile(r"(?:^|[/_.-])(?:macos|darwin|apple|cocoa)(?:[/_.-]|$)", re.IGNORECASE)}
PLATFORM_CODE = {
    "desktop-macos": re.compile(PLATFORM_CFG + r"|\bobjc2?\b|\bcocoa\b|\bappkit\b|core_?foundation|\bns(?:view|window|"
                                r"string|app|event|user_notification)|security_framework|\blaunchd?\b|\bplist\b",
                                re.IGNORECASE),
}
COMMENT = re.compile(r"^\s*(?://|/\*|\*)")

# Events that run iOS here: on main, ios.yml does (see the module docstring).
IOS_EVENTS = ("pull_request", "workflow_dispatch")


def changed_code(patch: str) -> str:
    """The added and removed lines of a diff, without comments."""
    lines = []
    for line in patch.splitlines():
        if line[:1] not in "+-" or line.startswith(("+++", "---")):
            continue
        code = line[1:]
        if COMMENT.match(code):
            continue
        lines.append(code.split("//", 1)[0])
    return "\n".join(lines)


def platform_specific(suite: str, files: list[dict]) -> bool:
    for f in files:
        path = f["filename"]
        if not path.startswith("crates/"):
            continue
        if PLATFORM_PATHS[suite].search(path):
            return True
        patch = f.get("patch")
        if patch is None:
            if path.endswith(".rs"):
                return True
            continue
        if path.endswith(".rs") and PLATFORM_CODE[suite].search(changed_code(patch)):
            return True
    return False


def paths_of(files: list[dict]) -> set[str]:
    return {f["filename"] for f in files} | {f["previous_filename"] for f in files if f.get("previous_filename")}


def select(files: list[dict] | None, tier: str, event: str,
           inputs: dict[str, tuple[list[str], set[str]]]) -> dict[str, bool]:
    """Which suites a change needs. files: [{"filename", "previous_filename"?, "patch"?}], or None for "everything"."""
    if files is None:
        suites = {suite: True for suite in SUITE_JOBS}
    else:
        paths = paths_of(files)
        if any(matches(p, CI_SELF) for p in paths):
            suites = {suite: True for suite in SUITE_JOBS}
        else:
            suites = {suite: any(reuse.covered(p, {suite: inputs[suite]}) for p in paths) for suite in SUITE_JOBS}
            if tier == "fast":
                for suite, fast_paths in FAST_PATHS.items():
                    if suites[suite]:
                        suites[suite] = (any(matches(p, fast_paths) for p in paths)
                                         or platform_specific(suite, [f for f in files
                                                                      if reuse.covered(f["filename"],
                                                                                       {suite: inputs[suite]})]))
    if event not in IOS_EVENTS:
        suites["ios"] = False
    return suites


def docker(files: list[dict] | None) -> bool:
    if files is None:
        return True
    return any(matches(p, DOCKER + CI_SELF) for p in paths_of(files))


def reused_suites(suites: dict[str, bool], passed: dict[str, set[str]]) -> list[str]:
    """Selected suites all of whose jobs passed on the same inputs."""
    return [suite for suite, on in suites.items()
            if on and SUITE_JOBS[suite] and all(job in passed.get(suite, set()) for job, _ in SUITE_JOBS[suite])]


def macos_jobs(running: dict[str, bool]) -> int:
    return sum(1 for suite, on in running.items() if on for _, runner in SUITE_JOBS[suite] if runner == "macos")


# GitHub API --------------------------------------------------------------------------------------------------------

def api(path: str):
    url = os.environ.get("GITHUB_API_URL", "https://api.github.com") + path
    request = urllib.request.Request(url, headers={
        "Authorization": f"Bearer {os.environ['GITHUB_TOKEN']}",
        "Accept": "application/vnd.github+json",
        "X-GitHub-Api-Version": "2022-11-28",
    })
    with urllib.request.urlopen(request, timeout=30) as response:
        return json.load(response)


def pull_request_files(repo: str, number: int, expected: int) -> list[dict] | None:
    files: list[dict] = []
    page = 1
    while True:
        batch = api(f"/repos/{repo}/pulls/{number}/files?per_page=100&page={page}")
        files.extend(batch)
        if len(batch) < 100:
            break
        page += 1
    # The API stops at 3000 files: run everything rather than guess.
    return files if len(files) >= expected else None


def compare_files(repo: str, base: str, head: str) -> list[dict] | None:
    if not base or set(base) == {"0"}:
        return None
    data = api(f"/repos/{repo}/compare/{base}...{head}")
    files = data.get("files")
    # compare lists at most 300 files.
    if files is None or len(files) >= 300:
        return None
    return files


def main() -> int:
    event = os.environ["GITHUB_EVENT_NAME"]
    repo = os.environ["GITHUB_REPOSITORY"]
    ref = os.environ.get("GITHUB_REF", "")
    with open(os.environ["GITHUB_EVENT_PATH"]) as f:
        payload = json.load(f)

    tier = "full"
    files: list[dict] | None = None
    why = ""
    trusted: set[str] = set()  # refs whose recorded passes this run may reuse
    try:
        if event == "pull_request":
            # The pull request as it is now (not as in the event), so that adding `ci:full` and re-running works.
            pr = api(f"/repos/{repo}/pulls/{payload['pull_request']['number']}")
            labels = {label["name"] for label in pr.get("labels", [])}
            if "ci:full" in labels:
                why = "label ci:full: every suite its files select, nothing reused"
            else:
                trusted = {ref, "refs/heads/main"}
                if os.environ.get("CI_PR_TIER", "fast") == "full":
                    why = "CI_PR_TIER=full"
                else:
                    tier = "fast"
            files = pull_request_files(repo, pr["number"], pr.get("changed_files", 0))
        elif event == "push":
            trusted = {"refs/heads/main"} if ref == "refs/heads/main" else set()
            files = compare_files(repo, payload.get("before", ""), payload.get("after", ""))
        # workflow_dispatch and the schedule: everything, nothing reused.
    except (OSError, KeyError, ValueError) as error:  # URLError, HTTPError, timeouts, bad JSON
        print(f"::warning::could not list the changed files ({error}); running every suite")
        files = None

    keys: dict[str, str] = {}
    extra: dict[str, list[str]] = {}
    inputs: dict[str, tuple[list[str], set[str]]] = {}
    try:
        index = reuse.tree_index(".")
        keys, extra, inputs = reuse.suite_keys(reuse.SUITES, index, reuse.Workspace("."))
    except (OSError, ValueError, KeyError, subprocess.CalledProcessError) as error:
        print(f"::warning::could not read the suites' inputs ({error}); running every suite, reusing nothing")
        files, trusted, keys = None, set(), {}
    selected = select(files, tier, event, inputs)
    if trusted and files is not None:
        uncovered = sorted(f["filename"] for f in files
                           if matches(f["filename"], reuse.BUILD_PATHS) and not reuse.covered(f["filename"], inputs))
        if uncovered:
            print(f"::notice::nothing reused: no suite's inputs cover {', '.join(uncovered[:5])}")
            why = (why + "; " if why else "") + "nothing reused (changed build files outside every suite's inputs)"
            trusted = set()
    passed: dict[str, set[str]] = {}
    if trusted:
        for suite, on in selected.items():
            if on and suite in keys:
                try:
                    passed[suite] = reuse.passed_jobs(api, repo, keys[suite], trusted)
                except (OSError, KeyError, ValueError) as error:
                    print(f"::warning::could not look up earlier passes of {suite} ({error})")
    reused = reused_suites(selected, passed)
    running = {suite: on and suite not in reused for suite, on in selected.items()}
    out = {
        "suites": json.dumps(running, separators=(",", ":")),
        "keys": json.dumps({s: k for s, k in keys.items() if running.get(s)}, separators=(",", ":")),
        "macos": str(macos_jobs(running)),
        "docker": json.dumps(docker(files)),
        "tier": tier,
        "pr": str(payload["pull_request"]["number"]) if event == "pull_request" else "",
    }
    with open(os.environ["GITHUB_OUTPUT"], "a") as f:
        for key, value in out.items():
            f.write(f"{key}={value}\n")

    lines = [f"### CI plan: {tier} tier{f' ({why})' if why else ''}", "",
             f"Changed files: {'all (not listed)' if files is None else len(files)}", "",
             "| suite | runs | |", "|---|---|---|"]
    for suite, on in selected.items():
        if suite == "ios" and event not in IOS_EVENTS:
            lines.append(f"| {suite} | – | ios.yml runs it on main |")
        elif not on:
            note = "not affected"
            if tier == "fast" and suite in FAST_PATHS and select(files, "full", event, inputs).get(suite):
                note = "fast tier: nothing macOS specific changed (add `ci:full` and re-run to run it)"
            lines.append(f"| {suite} | – | {note} |")
        elif suite in reused:
            lines.append(f"| {suite} | reused | passed on the same inputs |")
        else:
            lines.append(f"| {suite} | yes | |")
    lines += ["", f"macOS jobs: {out['macos']}"]
    reads = {path for paths in extra.values() for path in paths}
    if reads:
        lines += ["", "Files outside their packages that Rust suites read (part of their inputs): "
                  + ", ".join(f"`{p}`" for p in sorted(reads))]
    text = "\n".join(lines)
    print(text)
    summary = os.environ.get("GITHUB_STEP_SUMMARY")
    if summary:
        with open(summary, "a") as f:
            f.write(text + "\n")
    return 0


if __name__ == "__main__":
    sys.exit(main())
