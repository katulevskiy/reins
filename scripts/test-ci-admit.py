#!/usr/bin/env python3
"""Tests scripts/ci/admit.py: what holds macOS runners, who goes first, and when background work steps aside."""

from pathlib import Path
import sys
import unittest

ROOT = Path(__file__).resolve().parent.parent
sys.path.insert(0, str(ROOT / "scripts/ci"))
import admit  # noqa: E402

NOW = admit.stamp("2026-10-27T12:00:00Z")
ME = 1


def job(name, status="queued", labels=("ubuntu-24.04",), created="2026-10-27T11:00:00Z", completed=None,
        conclusion=None, job_id=None):
    return {"id": job_id or abs(hash((name, created))) % 10**9, "name": name, "status": status, "labels": list(labels),
            "created_at": created, "completed_at": completed, "conclusion": conclusion}


def mac(name="iOS", status="in_progress", created="2026-10-27T11:00:00Z", label="xcode-27"):
    return job(name, status, (label,), created)


def run(run_id, workflow="ci.yml", event="pull_request", status="in_progress", head="h"):
    return {"id": run_id, "path": f".github/workflows/{workflow}", "event": event, "status": status,
            "head_sha": head, "updated_at": "x"}


def snap(runs_jobs, **kw):
    runs = [r for r, _ in runs_jobs]
    jobs = {r["id"]: js for r, js in runs_jobs}
    return admit.snapshot_of(runs, jobs, ME, NOW, set(), **kw)


class ParseTest(unittest.TestCase):
    def test_gate_names(self):
        self.assertEqual(admit.parse_gate("macOS admission / admit [macos=2] #123"), (2, "normal", 123))
        self.assertEqual(admit.parse_gate("Yield to releases / admit [macos=1 bg]"), (1, "bg", None))
        self.assertIsNone(admit.parse_gate("admit [macos=1 front]"))  # main's CI has no gate
        self.assertEqual(admit.parse_gate("admit [macos=40] #1"), (admit.MAX_REQUEST, "normal", 1))
        self.assertIsNone(admit.parse_gate("iOS"))

    def test_macos_labels(self):
        self.assertTrue(admit.is_macos({"labels": ["xcode-27"]}, set()))
        self.assertTrue(admit.is_macos({"labels": ["macos-15"]}, set()))
        self.assertTrue(admit.is_macos({"labels": ["my-mac"]}, {"my-mac"}))
        self.assertFalse(admit.is_macos({"labels": ["ubuntu-24.04"]}, set()))
        self.assertFalse(admit.is_macos({"labels": ["windows-2025"]}, set()))


class SnapshotTest(unittest.TestCase):
    def test_busy_releases_and_reservations(self):
        s = snap([
            (run(2, "ios.yml", "push"), [job("Yield / admit [macos=1 bg]", "completed", conclusion="success",
                                             completed="2026-10-27T11:00:00Z"), mac("iOS build")]),
            (run(3, "release.yml", "push"), [job("Plan", "in_progress")]),  # macOS builds not created yet
            (run(4, "release.yml", "push"), [mac("macOS builds", "queued", label="macos-15")]),
            # admitted a minute ago, its two jobs not created yet
            (run(5), [job("macOS admission / admit [macos=2] #7", "completed", conclusion="success",
                          completed="2026-10-27T11:59:00Z")]),
            # admitted long ago: its reservation lapsed
            (run(6), [job("macOS admission / admit [macos=2] #8", "completed", conclusion="success",
                          completed="2026-10-27T10:00:00Z")]),
            (run(7, status="completed"), [mac()]),
            (run(ME), [mac("desktop-macos")]),  # my own jobs never count against me
        ])
        self.assertEqual(s.busy, 2)
        self.assertEqual(s.queued_foreground, 1)  # the release's queued build; ios.yml's run is background
        self.assertEqual(s.release, 1)
        self.assertEqual(s.unseen, 2)

    def test_reservation_counts_jobs_created_after_admission(self):
        s = snap([(run(5), [job("admit [macos=2] #7", "completed", conclusion="success",
                                completed="2026-10-27T11:59:00Z"),
                            mac("iOS", "queued", created="2026-10-27T11:59:30Z")])])
        self.assertEqual((s.busy, s.unseen), (1, 1))

    def test_skipped_jobs_hold_nothing(self):
        s = snap([(run(5), [dict(mac(), status="completed", conclusion="skipped")])])
        self.assertEqual(s.busy, 0)

    def test_waiters_and_claims(self):
        verified = {9}
        s = snap([
            (run(5, head="a"), [job("admit [macos=2] #9", "in_progress", created="2026-10-27T11:10:00Z")]),
            (run(6, head="b"), [job("admit [macos=1] #10", "in_progress", created="2026-10-27T11:05:00Z")]),
        ], pr_of=lambda r, claimed: claimed if claimed in verified else None)
        self.assertEqual([(w.run_id, w.pr, w.need) for w in s.waiters], [(5, 9, 2), (6, None, 1)])


def waiter(run_id, created, need=1, cls="normal", pr=None):
    return admit.Waiter(run_id, (created, run_id), need, cls, pr)


class DecideTest(unittest.TestCase):
    def test_first_come_first_served(self):
        s = admit.Snapshot(busy=3, waiters=[waiter(2, "10:00", 1), waiter(ME, "10:05", 1)])
        self.assertTrue(admit.decide(s.waiters[1], s, 5).admit)
        s.busy = 4  # one runner: the earlier run gets it
        self.assertFalse(admit.decide(s.waiters[1], s, 5).admit)
        self.assertTrue(admit.decide(s.waiters[0], s, 5).admit)

    def test_releases_come_first(self):
        s = admit.Snapshot(busy=4, release=1, waiters=[waiter(ME, "10:00")])
        self.assertFalse(admit.decide(s.waiters[0], s, 5).admit)
        s.release = 0
        self.assertTrue(admit.decide(s.waiters[0], s, 5).admit)

    def test_a_request_bigger_than_the_budget_goes_when_everything_is_free(self):
        s = admit.Snapshot(waiters=[waiter(ME, "10:00", 3)])
        self.assertTrue(admit.decide(s.waiters[0], s, 2).admit)

    def test_priority_and_lock(self):
        s = admit.Snapshot(busy=3, waiters=[waiter(2, "10:00", 2), waiter(ME, "10:05", 2, pr=9)], priority={9})
        self.assertTrue(admit.decide(s.waiters[1], s, 5).admit)
        self.assertFalse(admit.decide(s.waiters[0], s, 5).admit)
        # A lock keeps its runners whether or not its pull request uses them...
        lock = admit.Snapshot(busy=2, locked={9}, waiters=[waiter(ME, "10:00", 2)])
        self.assertFalse(admit.decide(lock.waiters[0], lock, 5).admit)
        lock.lock_use = {9: 2}
        lock.busy = 2
        self.assertTrue(admit.decide(lock.waiters[0], lock, 5).admit)
        # ...and its own runs go first, as soon as the runners are free.
        mine = admit.Snapshot(busy=3, locked={9}, waiters=[waiter(2, "09:00", 1), waiter(ME, "10:00", 2, pr=9)])
        self.assertTrue(admit.decide(mine.waiters[1], mine, 5).admit)

    def test_background_waits_for_everyone_and_leaves_headroom(self):
        s = admit.Snapshot(busy=2, waiters=[waiter(ME, "09:00", 3, "bg"), waiter(2, "10:00", 1)])
        self.assertFalse(admit.decide(s.waiters[0], s, 5).admit)  # a foreground run waits
        self.assertTrue(admit.decide(s.waiters[1], s, 5).admit)  # and is not held back by background
        s.waiters = s.waiters[:1]
        self.assertFalse(admit.decide(s.waiters[0], s, 5).admit)  # 2 + 3 + headroom > 5
        s.busy = 1
        self.assertTrue(admit.decide(s.waiters[0], s, 5).admit)

    def test_polling(self):
        self.assertEqual(admit.poll_interval(0, "normal", 5000), 60)
        self.assertEqual(admit.poll_interval(2, "normal", 5000), 180)
        self.assertEqual(admit.poll_interval(9, "normal", 5000), 300)
        self.assertEqual(admit.poll_interval(0, "bg", 5000), 120)
        self.assertEqual(admit.poll_interval(0, "normal", 900), 300)
        self.assertGreaterEqual(admit.QUOTA_RESERVE, 500)

    def test_own_request_merges_this_runs_gates(self):
        s = admit.Snapshot(waiters=[waiter(ME, "10:05", 1), waiter(ME, "10:01", 1), waiter(2, "10:03", 1)])
        me = admit.own_request(s, ME, 1, "normal")
        self.assertEqual((me.need, me.key[0]), (2, "10:01"))
        self.assertEqual(admit.decide(me, s, 5).position, 0)
        late = admit.own_request(admit.Snapshot(waiters=[waiter(2, "10:03")]), ME, 1, "normal")
        self.assertEqual(admit.decide(late, admit.Snapshot(waiters=[waiter(2, "10:03")]), 5).position, 1)


class WorkflowTest(unittest.TestCase):
    def test_names_it_reads(self):
        release = (ROOT / ".github/workflows/release.yml").read_text()
        # The release's macOS build, which a release run holds a runner for until it exists.
        self.assertIn(f"\n  macos:\n    name: {admit.RELEASE_MACOS_JOB}\n", release)
        self.assertIn("\n  ios-testflight:\n    name: TestFlight upload\n    needs:", release)
        self.assertEqual(admit.RELEASE_WORKFLOW, "release.yml")
        # Every workflow with macOS jobs or gates is read.
        for path in (ROOT / ".github/workflows").glob("*.yml"):
            text = path.read_text()
            if "macos-gate.yml" in text or "xcode-27" in text or "runs-on: macos" in text:
                self.assertIn(path.name, admit.WORKFLOWS | {"macos-gate.yml"}, path.name)
        # The gate's name is what every other gate parses.
        gate = (ROOT / ".github/workflows/macos-gate.yml").read_text()
        self.assertIn("name: \"admit [macos=${{ inputs.macos }}", gate)
        for name in ("admit [macos=2] #232", "admit [macos=1 bg]"):
            self.assertIsNotNone(admit.parse_gate(f"macOS admission / {name}"), name)


class FakeApi:
    def __init__(self, routes):
        self.routes = routes
        self.paths = []
        self.remaining = 5000
        self.requests = 0

    def get(self, path):
        self.paths.append(path)
        for prefix, value in self.routes.items():
            if path.startswith(prefix):
                return value(path) if callable(value) else value
        raise AssertionError(path)


class ReaderTest(unittest.TestCase):
    def test_one_runs_list_per_poll_and_runs_of_its_workflows_only(self):
        runs = [dict(run(5), workflow_id=1, created_at="2026-10-27T11:00:00Z"),
                dict(run(6, "cla.yml"), workflow_id=9, created_at="2026-10-27T11:00:00Z"),
                dict(run(7, status="completed"), workflow_id=1, created_at="2026-10-27T11:00:00Z")]
        api = FakeApi({
            "/repos/o/r/actions/workflows?": {"workflows": [{"id": 1, "path": ".github/workflows/ci.yml"},
                                                             {"id": 9, "path": ".github/workflows/cla.yml"}]},
            "/repos/o/r/actions/runs?": {"workflow_runs": runs},
            "/repos/o/r/actions/runs/5/jobs": {"jobs": [mac()]},
            "/repos/o/r/actions/runs/5": dict(run(5, status="completed"), workflow_id=1),
        })
        reader = admit.Reader(api, "o/r", set())
        self.assertEqual(reader.snapshot(ME, labels=False).busy, 1)
        self.assertEqual(sum("/actions/runs?" in p for p in api.paths), 1)
        self.assertFalse(any("/runs/6/" in p for p in api.paths))  # the CLA check's jobs are never read
        # Next poll: run 5 has dropped off the latest page; it is read on its own, once.
        api.routes["/repos/o/r/actions/runs?"] = {"workflow_runs": []}
        api.paths.clear()
        self.assertEqual(reader.snapshot(ME, labels=False).busy, 0)
        self.assertEqual(api.paths, ["/repos/o/r/actions/runs?per_page=100&page=1", "/repos/o/r/actions/runs/5"])

    def test_only_the_oldest_lock_holds(self):
        def events(path):
            number = int(path.split("/issues/", 1)[1].split("/", 1)[0])
            added = {3: "2026-10-27T09:00:00Z", 4: "2026-10-27T08:00:00Z"}[number]
            return [{"event": "labeled", "label": {"name": admit.LOCK_LABEL}, "created_at": added}]
        api = FakeApi({
            "/repos/o/r/actions/workflows?": {"workflows": []},
            "/repos/o/r/actions/runs?": {"workflow_runs": []},
            f"/repos/o/r/issues?state=open&labels={admit.quote(admit.LOCK_LABEL)}": [
                {"number": 3, "pull_request": {}}, {"number": 4, "pull_request": {}}],
            f"/repos/o/r/issues?state=open&labels={admit.quote(admit.PRIORITY_LABEL)}": [],
            "/repos/o/r/issues/": events,
        })
        saved = admit.time.time
        admit.time.time = lambda: NOW
        try:
            snap = admit.Reader(api, "o/r", set()).snapshot(ME)
        finally:
            admit.time.time = saved
        self.assertEqual((snap.locked, snap.priority), ({4}, {3}))


class FailOpenTest(unittest.TestCase):
    def test_anything_unexpected_lets_the_run_go(self):
        saved = admit.wait
        admit.wait = lambda: 1 / 0
        try:
            self.assertEqual(admit.main(), 0)
        finally:
            admit.wait = saved


class EmergencyTest(unittest.TestCase):
    def test_what_it_cancels_and_reruns(self):
        import emergency
        runs = [dict(run(1, head="mine"), status="in_progress"),
                dict(run(2, head="other"), status="queued"),
                dict(run(3, event="push", head="main"), status="in_progress"),
                dict(run(4, head="old"), status="completed")]
        self.assertEqual([r["id"] for r in emergency.to_cancel(runs, "mine")], [2])  # never main, never its own
        ios = [dict(run(5, "ios.yml", "push")), dict(run(6, "ios.yml", "schedule")),
               dict(run(7, "ios.yml", "workflow_dispatch"))]
        self.assertEqual([r["id"] for r in emergency.background(ios)], [5, 6])
        done = [dict(run(8, head="mine"), status="completed", conclusion="failure"),
                dict(run(9, head="mine"), status="completed", conclusion="success")]
        self.assertEqual(emergency.to_rerun(done[:1], "mine")["id"], 8)
        self.assertIsNone(emergency.to_rerun(done, "mine"))  # the latest passed
        self.assertIsNone(emergency.to_rerun(runs, "mine"))  # one is running

    def test_a_failed_label_removal_or_rerun_is_a_warning(self):
        import emergency
        calls = []

        def request(method, path, body=None):
            calls.append((method, path))
            if method == "DELETE" or path.endswith("/rerun"):
                raise OSError("HTTP 500")
            if path.endswith("/pulls/12"):
                return {"state": "open", "head": {"sha": "mine"}, "labels": [{"name": emergency.LOCK_LABEL}]}
            if "/runs?head_sha=" in path:
                return {"workflow_runs": [dict(run(8, head="mine"), status="completed", conclusion="failure")]}
            if "/runs?status=" in path:
                return {"workflow_runs": []}
            return None
        saved = (emergency.request, dict(emergency.os.environ))
        emergency.request = request
        emergency.os.environ.update({"GITHUB_REPOSITORY": "o/r", "PR": "12", "CANCEL_BACKGROUND": "false"})
        emergency.os.environ.pop("GITHUB_STEP_SUMMARY", None)
        try:
            self.assertEqual(emergency.main(), 0)
        finally:
            emergency.request = saved[0]
            emergency.os.environ.clear()
            emergency.os.environ.update(saved[1])
        self.assertIn(("POST", "/repos/o/r/issues/12/labels"), calls)
        self.assertIn(("POST", "/repos/o/r/actions/runs/8/rerun"), calls)


class YieldTest(unittest.TestCase):
    def test_background_steps_aside(self):
        self.assertEqual(admit.should_yield(admit.Snapshot()), [])
        self.assertTrue(admit.should_yield(admit.Snapshot(release=1)))
        self.assertTrue(admit.should_yield(admit.Snapshot(queued_foreground=1)))
        self.assertTrue(admit.should_yield(admit.Snapshot(waiters=[waiter(2, "10:00")])))
        self.assertEqual(admit.should_yield(admit.Snapshot(waiters=[waiter(2, "10:00", cls="bg")])), [])


if __name__ == "__main__":
    unittest.main()
