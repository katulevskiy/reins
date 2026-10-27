#!/usr/bin/env python3
"""The release gate must wait, match the tested commit, and reject failed or unavailable CI."""

import os
import re
import subprocess
import tempfile
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
SHA = "a" * 40

with tempfile.TemporaryDirectory(prefix="reins-ci-gate-") as directory:
    root = Path(directory)
    # A disposable gh supplies states and records the real gate's query arguments.
    gh = root / "gh"
    gh.write_text(
        "#!/usr/bin/env python3\n"
        "import os, pathlib, sys\n"
        "root = pathlib.Path(os.environ['GATE_TEST_ROOT'])\n"
        "(root / 'args').write_text('\\n'.join(sys.argv[1:]))\n"
        "states = (root / 'states').read_text().splitlines()\n"
        "state = states.pop(0) if states else 'missing'\n"
        "(root / 'states').write_text('\\n'.join(states))\n"
        "if state == 'api-error': sys.exit(1)\n"
        "print(state)\n"
    )
    gh.chmod(0o755)
    cases = [
        (["success"], True),
        (["missing", "queued", "success"], True),
        (["in_progress", "success"], True),
        (["failure"], False),
        (["cancelled"], False),
        (["timed_out"], False),
        (["skipped"], False),
        (["neutral"], False),
        (["action_required"], False),
        (["unknown"], False),
        (["api-error"], False),
        (["api-error", "success"], True),
        (["api-error"] * 3, False),
        (["missing"] * 3, False),
        (["in_progress"] * 3, False),
    ]
    environment = os.environ | {
        "PATH": f"{root}:{os.environ['PATH']}",
        "GATE_TEST_ROOT": str(root),
        "REINS_CI_POLL_ATTEMPTS": "3",
        "REINS_CI_POLL_SECONDS": "0",
    }
    for states, expected in cases:
        (root / "states").write_text("\n".join(states))
        result = subprocess.run(
            ["bash", str(ROOT / "scripts/wait-for-ci.sh"), SHA],
            env=environment,
            check=False,
            capture_output=True,
            text=True,
        )
        assert (result.returncode == 0) == expected, (
            states,
            result.stdout,
            result.stderr,
        )
        args = (root / "args").read_text().splitlines()
        for flag, value in (
            ("--commit", SHA),
            ("--workflow", "ci.yml"),
            ("--event", "push"),
            ("--branch", "main"),
        ):
            assert args[args.index(flag) + 1] == value
    result = subprocess.run(
        ["bash", str(ROOT / "scripts/wait-for-ci.sh"), "invalid"],
        env=environment,
        check=False,
        capture_output=True,
    )
    assert result.returncode != 0

workflow = (ROOT / ".github/workflows/release.yml").read_text()
publisher = workflow.split("  publish:\n", 1)[1]
gate = publisher.index('scripts/wait-for-ci.sh "$SHA"')
assert gate < publisher.index("docker buildx imagetools create")
assert gate < publisher.index("gh release create")
assert publisher.index("--draft\n") < publisher.index(
    'gh release edit "$TAG" --draft=false --latest'
)
assert "needs.android.result == 'success'" in publisher
# Publication waits for every build it needs, each of which must have succeeded: the Linux packages are release
# assets (and the feed's repositories), installed from on both architectures.
needs = publisher.split("    needs:\n", 1)[1].split("    if:", 1)[0]
needed = [line.strip().removeprefix("- ") for line in needs.splitlines() if line.strip()]
assert {"desktop-linux", "desktop", "desktop-app-linux", "desktop-app", "macos", "server", "docker",
        "android", "linux-packages", "linux-packages-amd64", "linux-packages-arm64"} <= set(needed), needed
# Every macOS build publication waits for is the one macos job; TestFlight comes after publication.
assert "ios-testflight" not in needed, needed
for job in needed[1:]:
    assert f"needs.{job}.result == 'success'" in publisher, job
# The Linux packages start once the Linux builds they package are done, not after every platform's.
packages = workflow.split("\n  linux-packages:\n", 1)[1].split("\n    steps:\n", 1)[0]
assert "needs: [plan, desktop-linux, desktop-app-linux]" in packages
# The AUR job runs after publication and can neither block nor fail the release.
aur = publisher.split("\n  aur:\n", 1)[1]
assert "needs: [plan, publish]" in aur and "continue-on-error: true" in aur
assert "needs.publish.result == 'success'" in aur
# Artifact retries must preserve the release gate: only the first attempt may fail softly.
action = (ROOT / ".github/actions/upload-artifact/action.yml").read_text()
first, retry = action.split("    - id: retry\n", 1)
assert "continue-on-error: true" in first
assert "continue-on-error:" not in retry
assert "steps.upload.outcome == 'failure'" in retry
assert "!cancelled()" in retry
assert "overwrite: true" in retry
assert action.count("if-no-files-found: error") == 2
for field in ("name", "path", "compression-level", "retention-days"):
    assert action.count(f"{field}: ${{{{ inputs.{field} }}}}") == 2
for job in ("desktop-linux", "desktop", "android"):
    section = workflow.split(f"\n  {job}:\n", 1)[1].split("\n\n  ", 1)[0]
    assert "fail-fast: false" in section, job
assert "uses: actions/upload-artifact@" not in workflow
assert workflow.count("uses: ./.github/actions/upload-artifact") == 13
# One macOS job before publication, on one runner: the CLI, the app's binaries and the notarized app. Its artifacts
# keep the names publication downloads (dist-*), and nothing else in the run runs on macOS before publication.
macos = workflow.split("\n  macos:\n", 1)[1].split("\n\n  server:\n", 1)[0]
assert "    name: macOS builds\n" in macos and "    needs: plan\n" in macos and "runs-on: macos-15" in macos
assert "name: dist-desktop-apple-darwin" in macos and "name: dist-app-macOS" in macos
assert "download-artifact" not in macos and "\n    environment:" not in macos
assert "timeout-minutes: 75 #" in macos and "timeout-minutes: 160" in macos
jobs = re.split(r"\n  ([a-z0-9-]+):\n", workflow.split("\njobs:\n", 1)[1])
macos_jobs = [name for name, body in zip(jobs[1::2], jobs[2::2])
              if re.search(r"^    runs-on: .*(macos|xcode)", body.split("\n    steps:", 1)[0], re.M)]
assert macos_jobs == ["macos", "ios-testflight"], macos_jobs
# The release never waits for CI's macOS gate.
assert "uses: ./.github/workflows/macos-gate.yml" not in workflow
# The APK and the Play bundle are built on two runners and must carry the same versionCode: plan's.
android = workflow.split("\n  android:\n", 1)[1].split("\n\n  ", 1)[0]
assert "VERSION_CODE: ${{ needs.plan.outputs.version-code }}" in android
assert "date -u" not in android
assert "needs.android.outputs.version-code" not in workflow
print("50 release CI gate and artifact retry checks passed")
