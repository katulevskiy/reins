#!/usr/bin/env python3
"""Exercise CI's actual component selectors against disposable Git histories."""

import os
from pathlib import Path
import re
import subprocess
import tempfile
import textwrap

ROOT = Path(__file__).resolve().parent.parent


def load_selector(workflow_name):
    # Read this literal shell block without introducing a YAML dependency in the lightweight checks job.
    workflow = (ROOT / ".github/workflows" / workflow_name).read_text()
    selector_step = workflow.split("      - name: Select build lanes\n", 1)[1]
    selector_lines = selector_step.split("        run: |\n", 1)[1].splitlines()
    body = []
    for line in selector_lines:
        if line.strip() and not line.startswith("          "):
            break
        body.append(line)
    return textwrap.dedent("\n".join(body))


CI_SELECTOR = load_selector("ci.yml")
IOS_SELECTOR = load_selector("ios.yml")

# (rust, android, docker, desktop, ios). ios is .github/workflows/ios.yml, not a ci.yml lane.
cases = [
    ("docs/example.md", (False, False, False, False, False)),
    ("README.md", (False, False, False, False, False)),
    ("src/config.rs", (True, False, False, False, False)),
    ("crates/reins-core/src/sso.rs", (True, True, False, True, True)),
    ("crates/reins-proto/src/lib.rs", (True, True, False, True, True)),
    ("crates/reins-policy/src/lib.rs", (True, True, False, True, True)),
    ("crates/reins-desktop/src/lib.rs", (True, False, False, True, False)),
    ("crates/reins-desktop-app/src/main.rs", (True, False, False, True, False)),
    ("crates/reins-e2e/tests/round_trip.rs", (True, False, False, False, False)),
    ("Cargo.toml", (True, True, False, True, False)),
    ("Cargo.lock", (True, True, False, True, True)),
    ("rust-toolchain.toml", (True, True, False, True, True)),
    (".cargo/config.toml", (True, True, False, True, True)),
    ("ios/README.md", (False, False, False, False, True)),
    ("ios/scripts/build-core.sh", (False, False, False, False, True)),
    ("android/app/build.gradle.kts", (False, True, False, False, False)),
    (".github/workflows/ci.yml", (True, True, True, True, False)),
    (".github/workflows/ios.yml", (False, False, False, False, True)),
    # The release's workflow and actions do not run in CI's lanes; the checks job lints them.
    (".github/workflows/release.yml", (False, False, False, False, False)),
    (".github/actions/signing/action.yml", (False, False, False, False, False)),
    ("docker/Dockerfile.debian", (False, False, True, False, False)),
    ("scripts/release-android.sh", (True, True, False, False, False)),
    ("scripts/install.ps1", (True, True, False, True, False)),
    # ci.yml treats scripts/* as rust and android. ios.yml names this file so the suite still runs.
    ("scripts/ios-ui-plan.py", (True, True, False, False, True)),
]

with tempfile.TemporaryDirectory(prefix="reins-ci-routing-") as directory:
    root = Path(directory)

    def git(*args):
        return subprocess.check_output(
            ["git", "-c", "user.email=test@example.com", "-c", "user.name=CI Test", *args],
            cwd=root,
            text=True,
            stderr=subprocess.DEVNULL,
        ).strip()

    def select(selector, keys, event, base):
        output = root / "outputs"
        output.unlink(missing_ok=True)
        environment = os.environ | {"EVENT": event, "BASE_SHA": base, "GITHUB_OUTPUT": str(output)}
        subprocess.run(
            ["bash", "-e", "-o", "pipefail", "-c", selector], cwd=root, env=environment, check=True
        )
        values = dict(line.split("=", 1) for line in output.read_text().splitlines())
        return tuple(values[key] == "true" for key in keys)

    def lanes(event, base):
        return select(CI_SELECTOR, ("rust", "android", "docker", "desktop"), event, base) + select(
            IOS_SELECTOR, ("ios",), event, base
        )

    git("init", "-q")
    (root / "baseline").write_text("base")
    git("add", ".")
    git("commit", "-qm", "base")
    baseline = git("rev-parse", "HEAD")
    for name, expected in cases:
        git("reset", "--hard", baseline)
        git("clean", "-fdq")
        path = root / name
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text("changed")
        git("add", ".")
        git("commit", "-qm", "change")
        actual = lanes("pull_request", baseline)
        assert actual == expected, (name, actual, expected)

    # Unreachable before-SHAs can follow history rewrites. Missing history must never skip validation.
    # The nightly iOS run and the weekly CI run select every lane, so a quiet week still refreshes caches.
    for event, base in [("push", "f" * 40), ("push", "0" * 40), ("push", ""), ("workflow_dispatch", baseline),
                        ("schedule", baseline), ("schedule", "")]:
        assert lanes(event, base) == (True, True, True, True, True), (event, base)

ios_workflow = (ROOT / ".github/workflows/ios.yml").read_text()
push = ios_workflow.split("\n  push:\n", 1)[1].split("\n  schedule:\n", 1)[0]
assert "branches: [main]" in push
for path in (
    "crates/reins-core/**",
    "crates/reins-proto/**",
    "crates/reins-policy/**",
    "Cargo.lock",
    "rust-toolchain.toml",
    ".cargo/**",
    "ios/**",
    ".github/workflows/ios.yml",
    "scripts/ios-ui-plan.py",
):
    assert path in push, path
assert 'cron: "17 3 * * *"' in ios_workflow
assert "41 5 * * 1" not in ios_workflow
assert "github.event_name != 'push'" not in ios_workflow
# A main push or a hand run must not take every macOS runner. The nightly still may.
assert "max-parallel: ${{ github.event_name == 'schedule' && 5 || 3 }}" in ios_workflow
# Pushes to main share one group, and a newer push cancels the older run; a newer push to a pull request cancels its
# older run too. Nightly and hand runs are never cancelled.
assert "github.event_name == 'push' && 'main'" in ios_workflow
assert "cancel-in-progress: ${{ github.event_name == 'push' || github.event_name == 'pull_request' }}" in ios_workflow
# A UI shard that max-parallel starts late asks again, and skips its tests while a release wants a macOS runner; the
# unit shard (it gates) never does. Every later step needs the plan, which a yielding shard skips.
shards = ios_workflow.split("\n  ios-test:\n", 1)[1].split("\n\n  # ", 1)[0]
yield_step = shards.split("      - name: Yield to a release\n", 1)[1].split("\n      - name: ", 1)[0]
assert "if: matrix.shard != 'unit'" in yield_step and "release.yml/runs" in yield_step
assert '"macOS builds"' in yield_step and '"TestFlight upload"' in yield_step and "skip=true" in yield_step
assert "if: steps.yield.outputs.skip != 'true'" in shards.split("      - name: Plan this shard\n", 1)[1].split("\n      - ", 1)[0]
assert shards.index("Yield to a release") < shards.index("Plan this shard")
for step in shards.split("\n      - ")[4:]:
    assert "steps.plan.outputs.only != ''" in step or "steps.tests.outcome" in step, step
assert "      actions: read" in shards.split("\n    steps:", 1)[0]
# ios-core.yml: a newer main push cancels an older one's build of the same variant; calls and hand runs never.
ios_core = (ROOT / ".github/workflows/ios-core.yml").read_text()
assert "format('ios-core-main-{0}', matrix.platform)" in ios_core
assert "cancel-in-progress: ${{ github.event_name == 'push' }}" in ios_core
# Each pull request has its own group, so unrelated pull requests never replace each other's pending iOS runs.
assert "format('pr-{0}', github.event.pull_request.number)" in ios_workflow
# A failed shard can be retried for as long as its result bundle is retained.
for artifact in ("ios-simulator-core", "ios-test-products"):
    block = ios_workflow.split(f"          name: {artifact}\n", 1)[1].split("\n\n", 1)[0]
    assert "retention-days: 7" in block, artifact
# A main push of this workflow must not satisfy the release gate. That gate names ci.yml only.
gate = (ROOT / "scripts/wait-for-ci.sh").read_text()
assert "--workflow ci.yml" in gate
assert "ios.yml" not in gate
# UI tests never gate a merge or a release: ReinsTests has its own shard, which gates the iOS check; the UI shards
# continue on error (the job, so the check's needs never see it, and the step, so the job stays green). A pull request
# runs the unit shard only (three macOS jobs: core, build, unit); the UI shards run on main, nightly and by hand.
assert ("shard: ${{ fromJSON(github.event_name == 'pull_request' && '[\"unit\"]' || "
        "'[\"unit\", \"autopilot\", \"grants\", \"onboarding\", \"field\", \"rest\"]') }}") in ios_workflow
assert ios_workflow.count("continue-on-error: ${{ matrix.shard != 'unit' }}") == 2
pull_request = ios_workflow.split("\n  pull_request:\n", 1)[1].split("\n  workflow_dispatch:", 1)[0]
assert "labeled" not in pull_request, pull_request
assert "github.event_name != 'pull_request'" in ios_workflow.split("- name: Select UI tests", 1)[1].split("run:", 1)[0]
# The agents' job and core workflows are tools: nothing waits for them.
for name in ("ios-job", "ios-core"):
    assert name not in gate, name
    assert "\n  pull_request:" not in (ROOT / ".github/workflows" / f"{name}.yml").read_text(), name

# Every macOS job of CI needs a Linux gate job that waits while a release waits for its macOS runner
# (.github/workflows/macos-gate.yml, with actions: read only); the release itself never waits for it.
GATE = "uses: ./.github/workflows/macos-gate.yml"
gated = 0
for name in ("ci", "ios", "ios-core", "ios-job"):
    text = (ROOT / ".github/workflows" / f"{name}.yml").read_text()
    parts = re.split(r"\n  ([a-z0-9-]+):\n", text.split("\njobs:\n", 1)[1])
    jobs = dict(zip(parts[1::2], parts[2::2]))
    gates = {job for job, body in jobs.items() if GATE in body}
    assert gates, name
    for job in gates:
        # Only ci.yml's gate may pass at once (on a push to main: that commit's release waits for CI). Every iOS gate
        # waits, on main too.
        assert name == "ci" or "wait:" not in jobs[job], (name, job)
        permissions = jobs[job].split("    permissions:\n", 1)[1].split("\n    with:", 1)[0]
        assert permissions.split("#", 1)[0].split() == ["actions:", "read"], (name, job, permissions)
    for job, body in jobs.items():
        head = body.split("\n    steps:", 1)[0]
        if re.search(r"^    runs-on: .*(macos|xcode|IOS_RUNNER)", head, re.M):
            needs = re.search(r"^    needs: (.*)$", head, re.M)
            assert needs and set(re.findall(r"[a-z0-9-]+", needs.group(1))) & gates, (name, job)
            gated += 1
assert gated == 6, gated
# ci.yml's gate passes at once on a push to main: the release of that commit waits for that run.
assert "wait: ${{ github.event_name != 'push' }}" in (ROOT / ".github/workflows/ci.yml").read_text()
# Publication waits for CI's whole result (scripts/wait-for-ci.sh): CI calls no other workflow that could wait or fail.
ci_text = (ROOT / ".github/workflows/ci.yml").read_text()
assert set(re.findall(r"uses: \./\.github/workflows/([a-z0-9-]+\.yml)", ci_text)) == {"macos-gate.yml"}
# ios-job.yml calls ios-core.yml, whose gate needs actions: read from its caller too.
ios_job = (ROOT / ".github/workflows/ios-job.yml").read_text()
assert "      actions: read" in ios_job.split("\n  core:\n", 1)[1].split("\n  run:\n", 1)[0]
release = (ROOT / ".github/workflows/release.yml").read_text()
assert GATE not in release
gate = (ROOT / ".github/workflows/macos-gate.yml").read_text()
assert "\n  workflow_call:\n" in gate and "runs-on: ubuntu-latest" in gate and "actions/checkout" not in gate
assert gate.split("\n    permissions:\n", 1)[1].split("\n    steps:", 1)[0].split("#", 1)[0].split() == ["actions:", "read"]

print(f"{len(cases) + 22} CI component-routing checks passed")
