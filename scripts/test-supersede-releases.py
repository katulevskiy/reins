#!/usr/bin/env python3
"""A newer release cancels older ones that have not reached publication, waits for one that is publishing, and never
cancels a run started by hand or one past publication."""

import importlib.util
import json
import os
import subprocess
import tempfile
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
spec = importlib.util.spec_from_file_location("supersede", ROOT / "scripts/supersede-releases.py")
supersede = importlib.util.module_from_spec(spec)
spec.loader.exec_module(supersede)
checks = 0


def check(condition, message):
    global checks
    checks += 1
    assert condition, message


def job(name, status="completed", conclusion="success"):
    return {"name": name, "status": status, "conclusion": conclusion}


def run(number, event="push", branch="main", status="in_progress"):
    return {"databaseId": 1000 + number, "number": number, "event": event, "status": status, "headBranch": branch,
            "headSha": f"{number:040d}"}


BUILDS = [job("Plan"), job("Desktop app (Windows-x64)", "in_progress", None), job("Android APK (full)")]

# The decision alone.
decide = supersede.decide
for jobs, older, event, expected in (
    ([], run(1), "push", "cancel"),  # queued: no job yet
    (BUILDS, run(1), "push", "cancel"),
    (BUILDS + [job("Linux packages (arm64 install test)")], run(1), "push", "cancel"),
    (BUILDS + [job("Publish the release", "queued", None)], run(1), "push", "wait"),
    (BUILDS + [job("Publish the release", "in_progress", None)], run(1), "push", "wait"),
    (BUILDS + [job("Publish the release", "in_progress", None)], run(1, "workflow_dispatch"), "push", "wait"),
    (BUILDS + [job("Publish the release"), job("Google Play upload", "in_progress", None)], run(1), "push", "leave"),
    (BUILDS + [job("Publish the release"), job("AUR (reins-bin)", "queued", None)], run(1), "push", "leave"),
    (BUILDS + [job("Publish the release", "completed", "skipped")], run(1), "push", "leave"),
    (BUILDS, run(1, "workflow_dispatch"), "push", "leave"),  # a manual release or verify_only run
    (BUILDS, run(1, branch="desktop/redesign"), "push", "leave"),
    (BUILDS, run(1), "workflow_dispatch", "leave"),  # only a newer push supersedes
):
    action, why = decide(older, jobs, event)
    check(action == expected and why, (jobs, older, event, action, expected))
check(supersede.PUBLISH_STAGE == {"Publish the release", "Google Play upload", "Homebrew cask", "AUR (reins-bin)"},
      "the publish stage")

# The whole script against a disposable gh: what it cancels, what it waits for, what it leaves.
with tempfile.TemporaryDirectory(prefix="reins-supersede-") as directory:
    root = Path(directory)
    gh = root / "gh"
    gh.write_text(
        "#!/usr/bin/env python3\n"
        "import json, os, pathlib, sys\n"
        "root = pathlib.Path(os.environ['TEST_ROOT'])\n"
        "state = json.loads((root / 'state.json').read_text())\n"
        "args = sys.argv[1:]\n"
        "with open(root / 'calls', 'a') as calls: calls.write(' '.join(args) + '\\n')\n"
        "if state.get('fail', 0) > 0:\n"
        "    state['fail'] -= 1; (root / 'state.json').write_text(json.dumps(state)); sys.exit(1)\n"
        "if args[:2] == ['run', 'list']:\n"
        "    assert args[args.index('--workflow') + 1] == 'release.yml' and args[args.index('--branch') + 1] == 'main'\n"
        "    print(json.dumps(state['runs']))\n"
        "elif args[:2] == ['run', 'view'] and args[-1] == 'status':\n"
        "    print(json.dumps({'status': state.get('status', 'in_progress')}))\n"
        "elif args[:2] == ['run', 'view']:\n"
        "    views = state['jobs'][args[2]]\n"
        "    print(json.dumps({'jobs': views[0]}))\n"
        "    if len(views) > 1: views.pop(0)\n"
        "    (root / 'state.json').write_text(json.dumps(state))\n"
        "elif args[:2] == ['run', 'cancel'] and state.get('cancel_fails'):\n"
        "    sys.exit(1)\n"
        "elif args[:2] == ['run', 'cancel']:\n"
        "    (root / 'cancelled').open('a').write(args[2] + '\\n')\n"
        "else:\n"
        "    sys.exit(2)\n"
    )
    gh.chmod(0o755)

    def script(runs, jobs, event="push", number=10, fail=0, **state):
        (root / "state.json").write_text(json.dumps({"runs": runs, "jobs": jobs, "fail": fail} | state))
        for name in ("calls", "cancelled"):
            (root / name).unlink(missing_ok=True)
        environment = os.environ | {
            "PATH": f"{root}:{os.environ['PATH']}", "TEST_ROOT": str(root), "REINS_RUN_ID": str(1000 + number),
            "REINS_RUN_NUMBER": str(number), "REINS_EVENT": event, "REINS_POLL_SECONDS": "0",
            "REINS_POLL_ATTEMPTS": "3", "REINS_GH_RETRY_SECONDS": "0",
        }
        result = subprocess.run(["python3", str(ROOT / "scripts/supersede-releases.py")], env=environment,
                                capture_output=True, text=True, check=False)
        cancelled = (root / "cancelled").read_text().split() if (root / "cancelled").exists() else []
        return result, cancelled

    publishing = BUILDS + [job("Publish the release", "in_progress", None)]
    published = BUILDS + [job("Publish the release")]
    runs = [run(10), run(9), run(8), run(7, "workflow_dispatch"), run(6), run(5, status="completed"), run(11)]
    jobs = {"1009": [BUILDS], "1008": [publishing, publishing, published], "1007": [BUILDS],
            "1006": [published + [job("Homebrew cask", "in_progress", None)]]}
    result, cancelled = script(runs, jobs)
    check(result.returncode == 0, result.stderr)
    check(cancelled == ["1009"], f"only the older push run that has not published is cancelled: {cancelled}")
    check("::notice::Cancelled release run #9" in result.stdout, result.stdout)
    check("Waiting for release run #8" in result.stdout and "#8 (1008, push, 00000000) has finished publishing"
          in result.stdout, result.stdout)
    check("Left release run #7" in result.stdout and "by hand" in result.stdout, "a manual run is left alone")
    check("Left release run #6" in result.stdout and "past publication" in result.stdout, "past publication")
    calls = (root / "calls").read_text()
    check("run view 1005" not in calls and "1011" not in calls and "1010" not in calls,
          "finished, newer and this run are not looked at")

    # Publication started between the first look and the cancel: the run is not cancelled.
    result, cancelled = script([run(10), run(9)], {"1009": [BUILDS, publishing, published]})
    check(result.returncode == 0 and cancelled == [], f"a run that began publishing is not cancelled: {cancelled}")

    # A manual release run only waits; it cancels nothing.
    result, cancelled = script([run(10, "workflow_dispatch"), run(9)], {"1009": [BUILDS]}, event="workflow_dispatch")
    check(result.returncode == 0 and cancelled == [], "a manual run cancels nothing")

    # A publication that does not end in time fails this job (the publish job's tag check is the backstop).
    result, cancelled = script([run(10), run(9)], {"1009": [publishing]})
    check(result.returncode == 1 and "still publishing" in result.stderr, result.stderr)

    # An API error is asked again; one that persists fails the job.
    result, cancelled = script([run(10), run(9)], {"1009": [BUILDS]}, fail=1)
    check(result.returncode == 0 and cancelled == ["1009"], f"one API error is retried: {result.stderr}")
    result, cancelled = script([run(10), run(9)], {"1009": [BUILDS]}, fail=5)
    check(result.returncode == 1 and cancelled == [] and "::error::gh run list" in result.stderr,
          "a persistent API error fails the job")

    # A run that finishes between the look and the cancel is not an error; one that cannot be cancelled is.
    result, cancelled = script([run(10), run(9)], {"1009": [BUILDS]}, cancel_fails=True, status="completed")
    check(result.returncode == 0 and "finished before it could be cancelled" in result.stdout, result.stderr)
    result, cancelled = script([run(10), run(9)], {"1009": [BUILDS]}, cancel_fails=True)
    check(result.returncode == 1 and "::error::gh run cancel" in result.stderr, result.stderr)

    result, cancelled = script([run(10)], {})
    check(result.returncode == 0 and "No older release run" in result.stdout, result.stdout)

# The workflow: only this job may cancel runs; plan waits for it; publications never overlap and refuse a taken tag.
workflow = (ROOT / ".github/workflows/release.yml").read_text()


def job_text(name):
    """A job's lines, up to the next job (or the comment above it)."""
    lines = workflow.split(f"\n  {name}:\n", 1)[1].splitlines()
    end = next((i for i, line in enumerate(lines) if line.startswith("  ") and not line.startswith("   ")), len(lines))
    return "\n".join(lines[:end])


job_section = job_text("supersede")
check("actions: write" in job_section and workflow.count("actions: write") == 1, "only supersede may cancel runs")
check("scripts/supersede-releases.py" in job_section, "supersede runs the script")
check("inputs.verify_only != true" in job_section, "verify_only runs cancel nothing")
plan = job_text("plan")
check("needs: supersede" in plan and "!cancelled()" in plan, "plan computes the version after supersede")
header = workflow.split("\njobs:\n", 1)[0]
check("cancel-in-progress: false" in header and "github.run_id" in header, "no workflow-wide cancelling or queueing")
publisher = job_text("publish")
check("group: ${{ inputs.verify_only && format('release-publish-verification-{0}', github.run_id) || "
      "'release-publish' }}" in publisher and "cancel-in-progress: false" in publisher, "one publication at a time")
guard = publisher.index('refs/tags/$TAG')
check(guard < publisher.index("docker buildx imagetools create") and guard < publisher.index("gh release create"),
      "the tag check comes before anything is published")
print(f"{checks} release supersede checks passed")
