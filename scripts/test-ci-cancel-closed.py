#!/usr/bin/env python3
"""Tests scripts/ci/cancel-closed.py and .github/workflows/cancel-closed-pr-runs.yml: a closed pull request's runs are
cancelled, and nothing else: never main's CI, a release, or a fork's branch of the same name."""

import importlib.util
import os
from pathlib import Path
import re
import unittest

ROOT = Path(__file__).resolve().parent.parent
spec = importlib.util.spec_from_file_location("cancel_closed", ROOT / "scripts/ci/cancel-closed.py")
cancel = importlib.util.module_from_spec(spec)
spec.loader.exec_module(cancel)


def run(run_id, workflow="ci.yml", event="pull_request", branch="feat/x", status="in_progress", repo="o/r"):
    return {"id": run_id, "path": f".github/workflows/{workflow}", "event": event, "head_branch": branch,
            "status": status, "head_repository": {"full_name": repo}}


class ToCancelTest(unittest.TestCase):
    def setUp(self):
        os.environ["GITHUB_REPOSITORY"] = "o/r"

    def test_only_the_closed_branchs_active_runs(self):
        runs = [
            run(1),  # CI of the pull request, running
            run(2, "ios.yml", "workflow_dispatch", status="queued"),  # a hand run on its branch
            run(3, "ios-job.yml", "workflow_dispatch", status="waiting"),
            run(4, "ios-core.yml", "workflow_call", status="pending"),
            run(5, status="completed"),  # finished
            run(6, branch="feat/y"),  # another pull request
            run(7, "release.yml", "push"),  # a release
            run(8, "ci.yml", "push"),  # a push run of that branch name: never
            run(9, repo="fork/r"),  # a fork's branch of the same name
            run(10, "cla.yml"),  # not a workflow it cancels
        ]
        self.assertEqual([r["id"] for r in cancel.to_cancel(runs, "feat/x", "o/r", "main")], [1, 2, 3, 4])

    def test_never_main(self):
        runs = [run(1, branch="main"), run(2, "ci.yml", "push", branch="main"), run(3, "release.yml", "push", "main")]
        self.assertEqual(cancel.to_cancel(runs, "main", "o/r", "main"), [])
        # A fork's pull request from its own `main`: only the fork's runs match, never this repository's main.
        fork = [run(4, branch="main", repo="fork/r")] + runs
        self.assertEqual([r["id"] for r in cancel.to_cancel(fork, "main", "fork/r", "main")], [4])
        self.assertEqual(cancel.to_cancel(runs, "", "o/r", "main"), [])


class WorkflowTest(unittest.TestCase):
    def test_workflow(self):
        text = (ROOT / ".github/workflows/cancel-closed-pr-runs.yml").read_text()
        on = text.split("\non:\n", 1)[1].split("\n\n", 1)[0]
        self.assertEqual(on.split(), ["pull_request:", "types:", "[closed]"])
        self.assertIn("\npermissions: {}\n", text)
        jobs = re.findall(r"^  ([a-z0-9-]+):\n", text.split("\njobs:\n", 1)[1], re.M)
        self.assertEqual(jobs, ["cancel"])
        permissions = text.split("    permissions:\n", 1)[1].split("\n    steps:", 1)[0]
        granted = sorted(line.split("#", 1)[0].strip() for line in permissions.splitlines() if line.strip())
        self.assertEqual(granted, ["actions: write", "contents: read"])
        # The untrusted branch name only reaches the script through the environment.
        self.assertNotIn("${{ github.event.pull_request.head.ref }}", text.split("run:", 1)[1])
        self.assertIn("persist-credentials: false", text)
        self.assertIn("if: github.event.pull_request.head.repo.full_name == github.repository", text)
        self.assertEqual(cancel.WORKFLOWS, {"ci.yml", "ios.yml", "ios-core.yml", "ios-job.yml"})
        for name in cancel.WORKFLOWS:
            self.assertTrue((ROOT / ".github/workflows" / name).exists(), name)


class SmokeQuarantineTest(unittest.TestCase):
    def test_the_trust_smoke_test_retries_once(self):
        ci = (ROOT / ".github/workflows/ci.yml").read_text()
        smoke = ci.split("\n  android-release-smoke:\n", 1)[1].split("\n  # ", 1)[0]
        self.assertIn("script: bash scripts/android-trust-smoke.sh", smoke)
        script = (ROOT / "scripts/android-trust-smoke.sh").read_text()
        self.assertEqual(script.count("\nsmoke\n"), 1)  # the one retry, after `smoke && exit 0`
        self.assertIn("smoke && exit 0", script)
        self.assertIn("trustSurvivesAsyncRefreshesAndItsSheetsStillWork", script)
        self.assertIn("class=dev.reins.android.TrustReleaseTest", script)


if __name__ == "__main__":
    unittest.main()
