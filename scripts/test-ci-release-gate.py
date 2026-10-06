#!/usr/bin/env python3
"""The release gate must wait, match the tested commit, and reject failed or unavailable CI."""

import os
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
desktop = workflow.split("  desktop:\n", 1)[1].split("  desktop-app:\n", 1)[0]
assert "fail-fast: false" in desktop
assert "uses: actions/upload-artifact@" not in workflow
assert workflow.count("uses: ./.github/actions/upload-artifact") == 8
print("31 release CI gate and artifact retry checks passed")
