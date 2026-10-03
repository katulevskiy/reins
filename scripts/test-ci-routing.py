#!/usr/bin/env python3
"""Exercise CI's actual component selector against disposable Git histories."""

import os
from pathlib import Path
import subprocess
import tempfile
import textwrap


ROOT = Path(__file__).resolve().parent.parent
workflow = (ROOT / ".github/workflows/ci.yml").read_text()
# Read this literal shell block without introducing a YAML dependency in the lightweight checks job.
selector_step = workflow.split("      - name: Select build lanes\n", 1)[1]
selector_lines = selector_step.split("        run: |\n", 1)[1].splitlines()
body = []
for line in selector_lines:
    if line.strip() and not line.startswith("          "):
        break
    body.append(line)
selector = textwrap.dedent("\n".join(body))

cases = [
    ("docs/example.md", (False, False, False)),
    ("README.md", (False, False, False)),
    ("src/config.rs", (True, False, False)),
    ("crates/reins-core/src/sso.rs", (True, True, False)),
    ("crates/reins-desktop/src/lib.rs", (True, False, False)),
    ("Cargo.lock", (True, True, False)),
    ("android/app/build.gradle.kts", (False, True, False)),
    (".github/actions/signing/action.yml", (True, True, True)),
    ("docker/Dockerfile.debian", (False, False, True)),
    ("scripts/release-android.sh", (True, True, False)),
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

    def select(event, base):
        output = root / "outputs"
        output.unlink(missing_ok=True)
        environment = os.environ | {"EVENT": event, "BASE_SHA": base, "GITHUB_OUTPUT": str(output)}
        subprocess.run(
            ["bash", "-e", "-o", "pipefail", "-c", selector], cwd=root, env=environment, check=True
        )
        values = dict(line.split("=", 1) for line in output.read_text().splitlines())
        return tuple(values[key] == "true" for key in ("rust", "android", "docker"))

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
        actual = select("pull_request", baseline)
        assert actual == expected, (name, actual, expected)

    # Unreachable before-SHAs can follow history rewrites. Missing history must never skip validation.
    for event, base in [("push", "f" * 40), ("push", "0" * 40), ("push", ""), ("workflow_dispatch", baseline)]:
        assert select(event, base) == (True, True, True), (event, base)

print(f"{len(cases) + 4} CI component-routing checks passed")
