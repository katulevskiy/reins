#!/usr/bin/env python3
"""Exercise CI's actual component selectors against disposable Git histories."""

import os
from pathlib import Path
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
# A pull request must not take every macOS runner. The nightly still may.
assert "max-parallel: ${{ github.event_name == 'schedule' && 5 || 3 }}" in ios_workflow
# Each PR owns its pending slot; only a newer run of that same PR may cancel it.
# Main/nightly/manual runs retain their event/commit groups without cancellation.
assert "group: ${{ github.workflow }}-${{ github.event_name == 'pull_request' && format('pr-{0}', github.event.pull_request.number) || format('{0}-{1}', github.event_name, github.sha) }}" in ios_workflow
assert "cancel-in-progress: ${{ github.event_name == 'pull_request' }}" in ios_workflow
# A failed shard can be retried for as long as its result bundle is retained.
for artifact in ("ios-simulator-core", "ios-test-products"):
    block = ios_workflow.split(f"          name: {artifact}\n", 1)[1].split("\n\n", 1)[0]
    assert "retention-days: 7" in block, artifact
# A main push of this workflow must not satisfy the release gate. That gate names ci.yml only.
gate = (ROOT / "scripts/wait-for-ci.sh").read_text()
assert "--workflow ci.yml" in gate
assert "ios.yml" not in gate

print(f"{len(cases) + 6} CI component-routing checks passed")
