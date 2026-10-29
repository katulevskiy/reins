#!/usr/bin/env python3
"""Tests scripts/ci/plan.py against this repository's real suite inputs, and checks ci.yml runs what it plans."""

import json
import os
from pathlib import Path
import re
import sys
import tempfile
import unittest

ROOT = Path(__file__).resolve().parent.parent
sys.path.insert(0, str(ROOT / "scripts/ci"))
import plan  # noqa: E402
import reuse  # noqa: E402

INDEX = reuse.tree_index(str(ROOT))
KEYS, EXTRA, INPUTS = reuse.suite_keys(reuse.SUITES, INDEX, reuse.Workspace(str(ROOT)))


def run(paths, tier="fast", event="pull_request", patches=None):
    files = [{"filename": p, **({"patch": patches[p]} if patches and p in patches else {})} for p in paths]
    return {suite for suite, on in plan.select(files, tier, event, INPUTS).items() if on}


LINUX_RUST = {"rust", "clippy"}


class SelectTest(unittest.TestCase):
    def test_docs_select_nothing(self):
        self.assertEqual(run(["docs/README.md", "README.md"]), {"packages"})  # the packages carry README.md
        self.assertEqual(run(["docs/architecture.md"]), set())
        self.assertEqual(run(["marketing/x.png", "design/next/board.png"]), set())

    def test_core_runs_server_android_ios_not_desktop(self):
        self.assertEqual(run(["crates/reins-core/src/sync.rs"]), LINUX_RUST | {"android", "ios"})
        self.assertEqual(run(["crates/reins-core/src/sync.rs"], "full"), LINUX_RUST | {"android", "ios"})

    def test_server(self):
        self.assertEqual(run(["src/api/core/mod.rs"]), LINUX_RUST)
        self.assertEqual(run(["migrations/sqlite/2026-10-01/up.sql"]), LINUX_RUST)
        self.assertEqual(run(["tests/reins_server/main.rs"]), LINUX_RUST)

    def test_policy_reaches_desktop_but_fast_tier_skips_macos(self):
        policy = "crates/reins-policy/src/engine.rs"
        plain = {policy: "@@\n+fn g() {}\n"}
        full = run([policy], "full", patches=plain)
        self.assertEqual(full, LINUX_RUST | {"replay", "desktop-linux", "desktop-macos", "desktop-windows",
                                             "android", "ios"})
        self.assertEqual(run([policy], patches=plain), full - {"desktop-macos"})
        # Unless the change is platform specific.
        patch = '@@\n+#[cfg(target_os = "macos")]\n+fn f() {}\n'
        self.assertIn("desktop-macos", run([policy], patches={policy: patch}))
        comment = "@@\n+// as on macOS, target_os decides\n"
        self.assertNotIn("desktop-macos", run([policy], patches={policy: comment}))
        # A Rust file without a diff (too large) counts as platform specific.
        self.assertIn("desktop-macos", run([policy]))

    def test_desktop_crates_run_every_desktop_suite(self):
        self.assertEqual(run(["crates/reins-desktop-app/src/tray.rs"]),
                         {"desktop-linux", "desktop-macos", "desktop-windows"})
        self.assertEqual(run(["crates/reins-desktop/src/daemon.rs"]),
                         LINUX_RUST | {"desktop-linux", "desktop-macos", "desktop-windows"})

    def test_platform_specific_files(self):
        self.assertTrue(plan.platform_specific("desktop-macos", [{"filename": "crates/reins-proto/src/macos.rs",
                                                                  "patch": "@@\n+fn f() {}\n"}]))
        self.assertTrue(plan.platform_specific("desktop-macos", [{"filename": "crates/reins-proto/src/lib.rs"}]))
        self.assertFalse(plan.platform_specific("desktop-macos", [{"filename": "android/app/x.kt"}]))

    def test_mobile(self):
        self.assertEqual(run(["android/app/src/main/java/dev/reins/android/MainActivity.kt"]), {"android"})
        self.assertEqual(run(["ios/Reins/App/ReinsApp.swift"]), {"ios"})
        self.assertEqual(run(["scripts/ios-ui-plan.py"]), {"ios"})
        self.assertEqual(run(["scripts/build-android-native.sh"]), {"android"})
        self.assertEqual(run(["scripts/android-trust-smoke.sh"]), {"android"})
        # The iOS workflows a pull request's iOS job shares the core and the runner setup with.
        for workflow in ("ios.yml", "ios-core.yml", "ios-job.yml"):
            self.assertEqual(run([f".github/workflows/{workflow}"]), {"ios"}, workflow)

    def test_ios_runs_here_on_pull_requests_manual_runs_and_main(self):
        self.assertIn("ios", run(["ios/Reins/App/ReinsApp.swift"], "full", "workflow_dispatch"))
        # The release waits for ci.yml on its commit (scripts/wait-for-ci.sh): main must run iOS here.
        self.assertIn("ios", run(["ios/Reins/App/ReinsApp.swift"], "full", "push"))
        self.assertTrue(all(plan.select(None, "full", "push", INPUTS).values()))
        everything = plan.select(None, "full", "schedule", INPUTS)
        self.assertFalse(everything["ios"])
        self.assertTrue(all(on for suite, on in everything.items() if suite != "ios"))
        self.assertTrue(plan.fallback("push", {})["suites"] and json.loads(plan.fallback("push", {})["suites"])["ios"])

    def test_scripts(self):
        self.assertEqual(run(["scripts/install.ps1"]), {"packages", "desktop-windows"})
        self.assertEqual(run(["scripts/package/linux-packages.sh"]), {"packages"})
        self.assertEqual(run(["scripts/release-android.sh"]), set())
        self.assertEqual(run([".github/workflows/release.yml"]), {"packages"})

    def test_ci_itself_and_build_files_run_everything(self):
        for path in (".github/workflows/ci.yml", ".github/workflows/macos-gate.yml", "scripts/ci/plan.py"):
            self.assertEqual(run([path]), set(plan.SUITE_JOBS), path)
        for path in ("Cargo.lock", "Cargo.toml", "rust-toolchain.toml", ".cargo/config.toml"):
            self.assertEqual(run([path]), set(plan.SUITE_JOBS) - {"packages"}, path)
        # Renames count from both sides.
        files = [{"filename": "docs/x.rs", "previous_filename": "crates/reins-core/src/x.rs"}]
        self.assertIn("ios", {s for s, on in plan.select(files, "fast", "pull_request", INPUTS).items() if on})

    def test_docker(self):
        self.assertTrue(plan.docker([{"filename": "docker/Dockerfile.debian"}]))
        self.assertTrue(plan.docker([{"filename": ".github/workflows/ci.yml"}]))
        self.assertTrue(plan.docker(None))
        self.assertFalse(plan.docker([{"filename": "src/main.rs"}]))


class UncoveredTest(unittest.TestCase):
    def test_an_uncovered_build_file_runs_every_rust_suite(self):
        self.assertEqual(run(["crates/reins-new/src/lib.rs"]), set(reuse.SUITE_PACKAGES))


class RunTest(unittest.TestCase):
    """plan_run and main against a fake API, on this repository's tree."""

    def setUp(self):
        self.saved = (plan.api, dict(os.environ), os.getcwd())
        os.chdir(ROOT)
        self.dir = tempfile.TemporaryDirectory(prefix="reins-ci-plan-", dir=os.environ.get("TMPDIR"))
        os.environ.update({"GITHUB_REPOSITORY": "o/r", "GITHUB_OUTPUT": f"{self.dir.name}/out",
                           "GITHUB_STEP_SUMMARY": f"{self.dir.name}/summary"})

    def tearDown(self):
        plan.api = self.saved[0]
        os.environ.clear()
        os.environ.update(self.saved[1])
        os.chdir(self.saved[2])
        self.dir.cleanup()

    def fake(self, files, passed_on_main):
        def api(path):
            if "/compare/" in path:
                return {"files": [{"filename": f} for f in files]}
            if "/actions/caches?" in path:
                key = path.split("key=", 1)[1].split("-&", 1)[0]
                suite = next(s for s in plan.SUITE_JOBS if key.startswith(f"ci-pass-v1-{s}-"))
                if suite in passed_on_main:
                    return {"actions_caches": [{"key": f"{key}-{job}", "ref": "refs/heads/main"}
                                               for job, _ in plan.SUITE_JOBS[suite]]}
                return {"actions_caches": []}
            raise AssertionError(path)
        plan.api = api

    def test_main_goes_by_fingerprint_not_by_diff(self):
        os.environ["GITHUB_REF"] = "refs/heads/main"
        # A docs-only push: a suite that has not passed on main on these inputs still runs (a red suite reruns on
        # every later push until it passes); one that has is reused.
        self.fake(["docs/README.md"], passed_on_main={"rust", "clippy", "packages"})
        out = plan.plan_run("push", {"before": "a" * 40, "after": "b" * 40})
        suites = json.loads(out["suites"])
        self.assertFalse(suites["rust"] or suites["clippy"] or suites["packages"])
        self.assertTrue(suites["android"] and suites["desktop-macos"] and suites["replay"])
        self.assertTrue(suites["ios"])
        self.assertEqual(out["macos"], "2")
        self.assertEqual(out["docker"], "false")

    def test_any_failure_runs_everything(self):
        os.environ["GITHUB_EVENT_NAME"] = "pull_request"
        os.environ["GITHUB_EVENT_PATH"] = f"{self.dir.name}/missing.json"
        plan.main()
        out = dict(line.split("=", 1) for line in Path(os.environ["GITHUB_OUTPUT"]).read_text().splitlines())
        self.assertTrue(all(json.loads(out["suites"]).values()))
        self.assertEqual((out["keys"], out["macos"], out["docker"]), ("{}", "2", "true"))

        def broken(path):
            raise RuntimeError("boom")
        plan.api = broken
        Path(f"{self.dir.name}/event.json").write_text('{"pull_request": {"number": 7}}')
        os.environ["GITHUB_EVENT_PATH"] = f"{self.dir.name}/event.json"
        Path(os.environ["GITHUB_OUTPUT"]).write_text("")
        plan.main()
        out = dict(line.split("=", 1) for line in Path(os.environ["GITHUB_OUTPUT"]).read_text().splitlines())
        self.assertTrue(all(json.loads(out["suites"]).values()))
        self.assertEqual(out["pr"], "7")


class ReuseAndRequestTest(unittest.TestCase):
    def test_reused_only_when_every_job_passed(self):
        suites = {s: True for s in plan.SUITE_JOBS}
        passed = {"rust": {"rust"}, "android": {"full", "play", "apk"}, "ios": {"ios"}}
        self.assertEqual(plan.reused_suites(suites, passed), ["rust", "ios"])
        every_android = {job for job, _ in plan.SUITE_JOBS["android"]}
        self.assertIn("android", plan.reused_suites(suites, {"android": every_android}))
        self.assertEqual(plan.reused_suites({"rust": False}, {"rust": {"rust"}}), [])

    def test_macos_request(self):
        self.assertEqual(plan.macos_jobs({s: True for s in plan.SUITE_JOBS}), 2)
        self.assertEqual(plan.macos_jobs({"ios": True, "rust": True}), 1)
        self.assertEqual(plan.macos_jobs({"rust": True}), 0)
        import admit
        self.assertGreaterEqual(admit.MAX_REQUEST, plan.macos_jobs({s: True for s in plan.SUITE_JOBS}))
        self.assertEqual(admit.LOCK_MACOS, plan.macos_jobs({s: True for s in plan.SUITE_JOBS}))


class WorkflowTest(unittest.TestCase):
    """ci.yml runs the suites plan.py knows, records each job's pass under the planned key, and requires them all."""

    def test_ci_yml(self):
        ci = (ROOT / ".github/workflows/ci.yml").read_text()
        recorded = set(re.findall(r"key: \$\{\{ fromJSON\(needs\.plan\.outputs\.keys\)\['([a-z-]+)'\] \}\}-([a-z0-9${}._ -]+)\n", ci))
        jobs = set()
        for suite, job in recorded:
            if "matrix.kind" in job:
                jobs |= {(suite, f"native-{kind}") for kind in ("arm64-v8a", "x86_64", "bindings")}
            elif "matrix.id" in job:
                jobs |= {(suite, lane) for lane in ("full", "play", "apk")}
            else:
                jobs.add((suite, job.strip()))
        expected = {(suite, job) for suite, js in plan.SUITE_JOBS.items() for job, _ in js}
        self.assertEqual(jobs, expected)
        for suite in plan.SUITE_JOBS:
            self.assertIn(f"fromJSON(needs.plan.outputs.suites)['{suite}']", ci, suite)
        result = ci.split("\n  result:\n", 1)[1]
        needs = re.search(r"needs: \[([^\]]+)\]", result).group(1)
        jobs_in_ci = set(re.findall(r"^  ([a-z0-9-]+):\n", ci.split("\njobs:\n", 1)[1], re.M)) - {"result"}
        self.assertEqual({n.strip() for n in needs.split(",")}, jobs_in_ci)
        self.assertIn("name: CI result", result)


if __name__ == "__main__":
    unittest.main()
