#!/usr/bin/env python3
"""CI's macOS gate (.github/workflows/macos-gate.yml) waits while a release waits for a macOS runner, and never long."""

import json
import os
from pathlib import Path
import subprocess
import tempfile
import textwrap

ROOT = Path(__file__).resolve().parent.parent
workflow = (ROOT / ".github/workflows/macos-gate.yml").read_text()
script = textwrap.dedent(workflow.split("        shell: python3 {0}\n        run: |\n", 1)[1])
release = (ROOT / ".github/workflows/release.yml").read_text()
# The job names the gate looks for are release.yml's.
assert '\n  macos:\n    name: macOS builds\n' in release
assert '\n  ios-testflight:\n    name: TestFlight upload\n' in release
assert 'BUILDS = "macOS builds"' in script and 'UPLOAD = "TestFlight upload"' in script

checks = 0


def run(snapshots, minutes="60", errors_at=()):
    """Run the gate against release runs that change on each poll. A snapshot: [(run status, {job: status})]."""
    global checks
    checks += 1
    with tempfile.TemporaryDirectory(prefix="reins-macos-gate-") as directory:
        root = Path(directory)
        (root / "gate.py").write_text(script)
        (root / "snapshots.json").write_text(json.dumps({"snapshots": snapshots, "polls": 0, "errors": list(errors_at)}))
        gh = root / "gh"
        gh.write_text(textwrap.dedent("""\
            #!/usr/bin/env python3
            import json, pathlib, sys
            state_path = pathlib.Path(__file__).with_name("snapshots.json")
            state = json.loads(state_path.read_text())
            path = sys.argv[2]
            if path.startswith("repos/o/r/actions/workflows/release.yml/runs"):
                state["polls"] += 1
                state_path.write_text(json.dumps(state))
                if state["polls"] in state["errors"]:
                    print("HTTP 502", file=sys.stderr)
                    sys.exit(1)
            snapshots = state["snapshots"]
            current = snapshots[min(max(state["polls"], 1), len(snapshots)) - 1] if snapshots else []
            if "/workflows/" in path:
                print(json.dumps({"workflow_runs": [
                    {"id": i + 1, "run_number": i + 10, "status": status, "head_sha": "ab" * 20}
                    for i, (status, _) in enumerate(current)]}))
            else:
                run_id = int(path.split("/runs/", 1)[1].split("/", 1)[0])
                jobs = current[run_id - 1][1]
                print(json.dumps({"jobs": [{"name": name, "status": status} for name, status in jobs.items()]}))
            """))
        gh.chmod(0o755)
        env = os.environ | {
            "PATH": f"{root}:{os.environ['PATH']}",
            "GH_REPO": "o/r",
            "REINS_GATE_MINUTES": minutes,
            "REINS_GATE_POLL_SECONDS": "0",
        }
        result = subprocess.run(["python3", str(root / "gate.py")], env=env, capture_output=True, text=True,
                                timeout=60, check=False)
        polls = json.loads((root / "snapshots.json").read_text())["polls"]
        assert result.returncode == 0, result.stderr
        return polls, result.stdout


BUILDS, UPLOAD, PLAN = "macOS builds", "TestFlight upload", "Plan"

# No release run, or only finished ones: CI goes on at once.
polls, out = run([[]])
assert polls == 1 and "No release waits" in out, out
polls, out = run([[("completed", {PLAN: "completed", BUILDS: "completed"})]])
assert polls == 1 and "Waiting" not in out, out

# A release whose macOS job is not created yet (plan running), then queued, then running: CI waits for the runner.
polls, out = run([
    [("queued", {})],
    [("in_progress", {PLAN: "in_progress"})],
    [("in_progress", {PLAN: "completed", BUILDS: "queued"})],
    [("in_progress", {PLAN: "completed", BUILDS: "in_progress"})],
])
assert polls == 4 and "is not created yet" in out and "'macOS builds' is queued" in out, out
assert "::notice::Waited" in out, out

# A release that releases nothing creates the macOS job skipped (completed): no wait.
polls, out = run([[("in_progress", {PLAN: "completed", BUILDS: "completed", "Publish the release": "queued"})]])
assert polls == 1, out

# After publication, a TestFlight upload waiting for a runner holds CI back too; once it runs, CI goes on.
polls, out = run([
    [("in_progress", {BUILDS: "completed", UPLOAD: "queued"})],
    [("in_progress", {BUILDS: "completed", UPLOAD: "in_progress"})],
])
assert polls == 2 and "'TestFlight upload' is queued" in out, out

# Two releases: CI waits for both.
polls, out = run([
    [("in_progress", {BUILDS: "in_progress"}), ("in_progress", {BUILDS: "queued"})],
    [("in_progress", {BUILDS: "completed"}), ("in_progress", {BUILDS: "in_progress"})],
])
assert polls == 2, out

# Never longer than the limit: then CI goes on with a warning.
polls, out = run([[("in_progress", {BUILDS: "queued"})]], minutes="0")
assert polls == 1 and "::warning::A release still waits" in out, out
polls, out = run([[("in_progress", {BUILDS: "queued"})]], minutes="0.001")
assert polls >= 1 and "::warning::A release still waits" in out, out

# An API error is asked again; five in a row and CI goes on.
polls, out = run([[("in_progress", {BUILDS: "queued"})], []], errors_at=(2,))
assert polls == 3 and "Could not read the release runs (1 of 5)" in out and "No release waits" in out, out
polls, out = run([[("in_progress", {BUILDS: "queued"})]], errors_at=(1, 2, 3, 4, 5))
assert polls == 5 and "::warning::The release runs could not be read" in out, out

print(f"{checks + 4} macOS gate checks passed")
