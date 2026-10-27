#!/usr/bin/env python3
"""Admission for macOS runners: a run's macOS jobs start only when runners are free for all of them.

The account runs five GitHub-hosted macOS jobs at once, shared by CI (ci.yml), the iOS runs on main (ios.yml), the
iOS jobs agents dispatch (ios-job.yml, ios-core.yml) and releases (release.yml). Linux and Windows runners never queue
here (docs/reference/ci.md), so only macOS is admitted. Every macOS job of those workflows needs a gate job
(.github/workflows/macos-gate.yml) that runs this script and waits until the macOS runners its jobs need fit.

Bookkeeping lives in GitHub itself, so nothing needs write access (pull requests from forks get a read-only token):

  - A gate's request is in its job name: "admit [macos=2] #123" (a pull request's CI run), "admit [macos=1 bg]"
    (background: ios.yml on main and at night), "admit [macos=1 front]" (main's CI, which never waits).
  - A gate whose job is not finished is waiting. Waiters are served by rank, then first come, first served (the
    gate job's creation time).
  - Busy: every macOS job (labels macos-* or xcode-*, or CI_MACOS_LABELS) queued or running in any active run.
  - A gate that let its run go reserves its request until the jobs it gated exist (UNSEEN_GRACE at most).

Releases come first. A release run whose "macOS builds" job does not exist yet (it is planning or superseding) holds
one runner; once that job exists it is busy like any other, queued or running. Release runs are never gated.

Ranks: main's CI (front: the release of that commit waits for it, so it never waits), then pull requests labelled
`ci:lock`, then `ci:priority`, then everything else, then background. A waiter goes when busy + reserved + what the
waiters ahead of it need + its own request fits in the budget (CI_BUDGET_MACOS, 5): nobody is overtaken on the runners
they wait for. Background runs (ios.yml's UI tests, which never gate a merge or a release) go only when nobody else
waits, and leave HEADROOM runners free; their UI shards ask again on their runner (`--yield`) and skip their tests while
anyone else wants a macOS runner.

Labels need the triage role or above to add, so GitHub decides who can use them. A run claims its pull request in its
gate's name; the claim counts only when that pull request's head is the run's commit.

  - `ci:priority`: the pull request's runs go to the front of the queue (behind locked ones and releases).
  - `ci:lock`: while the pull request is open, LOCK_MACOS runners are kept for it whether or not it runs anything, and
    its runs go first. It ends when the label goes, the pull request is merged or closed, or CI_LOCK_HOURS (6) after
    the label was added. ci-emergency.yml adds it and cancels the other pull requests' runs.

A pull request can edit its own copy of the workflows, so a request read from another run is capped at MAX_REQUEST.

Admission never blocks CI: it lets the run go on API errors (five in a row), when the rate limit resets more than ten
minutes away, or after MAX_WAIT_MINUTES. It uses conditional requests (a 304 does not count against the GITHUB_TOKEN's
rate limit), refetches a run's jobs only when the run changed, and polls less often the further back it is.

  admit.py            wait (environment: CI_MACOS, CI_CLASS normal|bg|front, CI_PR, CI_BUDGET_MACOS, ...)
  admit.py --yield    print skip=true to GITHUB_OUTPUT when a background job should step aside
"""

from __future__ import annotations

import calendar
import json
import os
import re
import sys
import time
import urllib.error
import urllib.request
from dataclasses import dataclass, field
from urllib.parse import quote

GATE = re.compile(r"\badmit \[macos=(\d+)(?: (bg|front))?\](?: #(\d+))?")
MAX_REQUEST = 3  # ios.yml's tests gate; a pull request's CI asks for at most 2 (desktop-macos and iOS)
LOCK_MACOS = 2  # a pull request's whole CI: desktop-macos and iOS
HEADROOM = 1  # runners background runs leave free
UNSEEN_GRACE = 10 * 60  # an admitted gate's jobs are created within seconds
RELEASE_WORKFLOW = "release.yml"
RELEASE_MACOS_JOB = "macOS builds"
ACTIVE = {"requested", "waiting", "pending", "queued", "in_progress"}
NO_RUNNER = {"requested", "waiting", "pending", "queued"}
PRIORITY_LABEL = "ci:priority"
LOCK_LABEL = "ci:lock"
LOCK_HOURS = 6
RANK = {"lock": 0, "priority": 1, "normal": 2, "bg": 3}


def stamp(timestamp: str | None) -> float:
    if not timestamp:
        return 0
    return float(calendar.timegm(time.strptime(timestamp.replace(".000", "")[:19], "%Y-%m-%dT%H:%M:%S")))


def is_macos(job: dict, extra_labels: set[str]) -> bool:
    for label in job.get("labels") or []:
        label = label.lower()
        if label.startswith(("macos", "xcode")) or label in extra_labels:
            return True
    return False


def parse_gate(name: str) -> tuple[int, str, int | None] | None:
    """(request, class, claimed pull request) from a gate job's name."""
    match = GATE.search(name)
    if not match:
        return None
    return min(int(match.group(1)), MAX_REQUEST), match.group(2) or "normal", int(match.group(3)) if match.group(3) else None


@dataclass
class Waiter:
    run_id: int
    key: tuple  # (created_at, job id): queue order
    need: int
    cls: str  # normal, bg or front
    pr: int | None = None  # its pull request, when verified


@dataclass
class Snapshot:
    busy: int = 0  # macOS jobs queued or running
    queued_foreground: int = 0  # of those, queued in runs that are not background
    unseen: int = 0  # admitted requests whose jobs are not created yet
    release: int = 0  # release runs whose macOS build is not created yet
    waiters: list[Waiter] = field(default_factory=list)
    lock_use: dict[int, int] = field(default_factory=dict)  # locked pull request -> macOS jobs its runs use
    priority: set[int] = field(default_factory=set)
    locked: set[int] = field(default_factory=set)
    lines: list[str] = field(default_factory=list)  # what holds runners, for the log


def snapshot_of(runs: list[dict], jobs: dict[int, list[dict]], me: int, now: float, extra_labels: set[str],
                pr_of=lambda run, claimed: None) -> Snapshot:
    """What every active run holds and waits for. `jobs`: run id -> its jobs. `pr_of(run, claimed)` verifies a claim."""
    snap = Snapshot()
    for run in runs:
        if run.get("status") not in ACTIVE:
            continue
        run_jobs = jobs.get(run["id"], [])
        workflow = (run.get("path") or "").rsplit("/", 1)[-1]
        if workflow == RELEASE_WORKFLOW and run["id"] != me:
            if not any(j["name"] == RELEASE_MACOS_JOB for j in run_jobs):
                snap.release += 1
                snap.lines.append(f"release run {run['id']}: {RELEASE_MACOS_JOB!r} is not created yet")
        gates = []
        for job in run_jobs:
            parsed = parse_gate(job["name"])
            if parsed:
                gates.append((job, *parsed))
        background = any(cls == "bg" for _, _, cls, _ in gates)
        claimed = next((pr for _, _, _, pr in gates if pr is not None), None)
        pr = pr_of(run, claimed) if claimed is not None else None
        mac = [j for j in run_jobs if is_macos(j, extra_labels) and j.get("conclusion") != "skipped"]
        if run["id"] != me:
            active = [j for j in mac if j.get("status") in ACTIVE]
            snap.busy += len(active)
            if not background:
                snap.queued_foreground += sum(1 for j in active if j.get("status") in NO_RUNNER)
            if pr is not None:
                snap.lock_use[pr] = snap.lock_use.get(pr, 0) + len(active)
            if active:
                snap.lines.append(f"{workflow} run {run['id']}: {len(active)} macOS job(s) "
                                  f"({', '.join(sorted(j['status'] for j in active))})")
        for job, need, cls, _ in gates:
            if job.get("status") != "completed":
                snap.waiters.append(Waiter(run["id"], (job.get("created_at") or "", job["id"]), need, cls, pr))
            elif job.get("conclusion") == "success" and run["id"] != me:
                since = now - stamp(job.get("completed_at"))
                if since < UNSEEN_GRACE:
                    after = [j for j in mac if stamp(j.get("created_at")) >= stamp(job.get("completed_at"))]
                    snap.unseen += max(0, need - len(after))
    return snap


@dataclass
class Decision:
    admit: bool
    position: int
    used: int
    ahead: int


def rank(w: Waiter, snap: Snapshot) -> int:
    if w.pr is not None and w.pr in snap.locked:
        return RANK["lock"]
    if w.pr is not None and w.pr in snap.priority:
        return RANK["priority"]
    return RANK["bg"] if w.cls == "bg" else RANK["normal"]


def own_request(snap: Snapshot, run_id: int, need: int, cls: str) -> Waiter:
    """This run's request. A run may have two gates waiting at once (ios-job.yml's, and ios-core.yml's for a core it
    builds): they are one request, the sum of both, queued as early as the earlier one. A gate not listed yet (the API
    lags) queues last."""
    mine = [w for w in snap.waiters if w.run_id == run_id]
    if not mine:
        return Waiter(run_id, ("~", 0), need, cls)
    return Waiter(run_id, min(w.key for w in mine), max(need, sum(w.need for w in mine)), cls,
                  next((w.pr for w in mine if w.pr is not None), None))


def decide(me: Waiter, snap: Snapshot, budget: int) -> Decision:
    if me.cls == "front":
        return Decision(True, 0, 0, 0)
    used = snap.busy + snap.unseen + snap.release
    # A lock keeps LOCK_MACOS runners whether or not its pull request uses them.
    for pr in snap.locked:
        if pr != me.pr:
            used += max(0, LOCK_MACOS - snap.lock_use.get(pr, 0))
    need = min(me.need, budget)
    my_rank = rank(me, snap)
    if my_rank == RANK["lock"]:
        return Decision(used + need <= budget, 0, used, 0)
    others = [w for w in snap.waiters if w.cls != "front" and w.run_id != me.run_id]
    ahead = 0
    position = 0
    for w in others:
        if (rank(w, snap), w.key) < (my_rank, me.key):
            position += 1
            if rank(w, snap) != RANK["lock"]:  # a lock's runners are already in `used`
                ahead += w.need
    if my_rank == RANK["bg"]:
        foreground = any(rank(w, snap) < RANK["bg"] for w in others)
        need = min(me.need, max(1, budget - HEADROOM))
        return Decision(not foreground and used + ahead + need + HEADROOM <= budget, position, used, ahead)
    return Decision(used + ahead + need <= budget, position, used, ahead)


def should_yield(snap: Snapshot) -> list[str]:
    """Why a background job should step aside now (nothing: go on)."""
    why = []
    if snap.release:
        why.append(f"{snap.release} release run(s) will want a macOS runner")
    if snap.queued_foreground:
        why.append(f"{snap.queued_foreground} macOS job(s) of CI, releases or iOS jobs wait for a runner")
    waiting = [w for w in snap.waiters if w.cls != "bg"]
    if waiting:
        why.append(f"{len(waiting)} run(s) wait for admission")
    return why


# GitHub API --------------------------------------------------------------------------------------------------------

class RateLimited(Exception):
    def __init__(self, wait: float):
        super().__init__(f"rate limited for {wait:.0f}s")
        self.wait = wait


class Api:
    def __init__(self, token: str, base: str):
        self.token = token
        self.base = base
        self.cache: dict[str, tuple[str, object]] = {}
        self.remaining = 1000
        self.requests = 0

    def get(self, path: str):
        url = self.base + path
        headers = {
            "Authorization": f"Bearer {self.token}",
            "Accept": "application/vnd.github+json",
            "X-GitHub-Api-Version": "2022-11-28",
        }
        cached = self.cache.get(url)
        if cached:
            headers["If-None-Match"] = cached[0]
        request = urllib.request.Request(url, headers=headers)
        try:
            with urllib.request.urlopen(request, timeout=30) as response:
                self._note(response.headers, counted=True)
                body = json.load(response)
                etag = response.headers.get("ETag")
                if etag:
                    self.cache[url] = (etag, body)
                return body
        except urllib.error.HTTPError as error:
            self._note(error.headers, counted=error.code != 304)
            if error.code == 304 and cached:
                return cached[1]
            if error.code in (403, 429):
                retry = error.headers.get("Retry-After")
                reset = error.headers.get("X-RateLimit-Reset")
                if retry:
                    raise RateLimited(float(retry)) from error
                if error.headers.get("X-RateLimit-Remaining") == "0" and reset:
                    raise RateLimited(max(30.0, float(reset) - time.time())) from error
            raise

    def _note(self, headers, counted: bool) -> None:
        if headers is None:
            return
        remaining = headers.get("X-RateLimit-Remaining")
        if remaining is not None:
            self.remaining = int(remaining)
        if counted:
            self.requests += 1


class Reader:
    """Active runs and their jobs, refetching a run's jobs only when the run changed."""

    REFRESH = 300  # refetch an unchanged run's jobs at least this often (seconds)

    def __init__(self, api: Api, repo: str, extra_labels: set[str]):
        self.api = api
        self.repo = repo
        self.extra_labels = extra_labels
        self.jobs: dict[int, tuple[str, float, list[dict]]] = {}
        self.heads: dict[int, str] = {}

    def run_jobs(self, run: dict) -> list[dict]:
        cached = self.jobs.get(run["id"])
        now = time.time()
        if cached and cached[0] == run["updated_at"] and now - cached[1] < self.REFRESH:
            return cached[2]
        jobs: list[dict] = []
        page = 1
        while True:
            data = self.api.get(f"/repos/{self.repo}/actions/runs/{run['id']}/jobs?filter=latest&per_page=100&page={page}")
            jobs.extend(data["jobs"])
            if len(data["jobs"]) < 100:
                break
            page += 1
        self.jobs[run["id"]] = (run["updated_at"], now, jobs)
        return jobs

    def runs(self) -> list[dict]:
        runs: list[dict] = []
        for status in ("in_progress", "queued"):
            page = 1
            while True:
                data = self.api.get(f"/repos/{self.repo}/actions/runs?status={status}&per_page=100&page={page}")
                runs.extend(data["workflow_runs"])
                if len(data["workflow_runs"]) < 100:
                    break
                page += 1
        # Workflows without macOS jobs or gates (the CLA check, the cache prune) need no jobs list.
        return [r for r in runs if (r.get("path") or "").rsplit("/", 1)[-1] in WORKFLOWS]

    def snapshot(self, me: int, labels: bool = True) -> Snapshot:
        runs = self.runs()
        live = {run["id"] for run in runs}
        self.jobs = {k: v for k, v in self.jobs.items() if k in live}
        jobs = {run["id"]: self.run_jobs(run) for run in runs}
        priority = self.labelled(PRIORITY_LABEL) if labels else set()
        locked = self.locks() if labels else set()
        special = priority | locked

        def pr_of(run: dict, claimed: int) -> int | None:
            if claimed not in special or run.get("event") != "pull_request":
                return None
            if claimed not in self.heads:
                self.heads[claimed] = self.api.get(f"/repos/{self.repo}/pulls/{claimed}").get("head", {}).get("sha", "")
            return claimed if self.heads[claimed] == run.get("head_sha") else None

        self.heads = {}
        snap = snapshot_of(runs, jobs, me, time.time(), self.extra_labels, pr_of)
        snap.priority = priority
        snap.locked = locked
        return snap

    def labelled(self, label: str) -> set[int]:
        found: set[int] = set()
        page = 1
        while True:
            issues = self.api.get(f"/repos/{self.repo}/issues?state=open&labels={quote(label)}&per_page=100&page={page}")
            found |= {i["number"] for i in issues if "pull_request" in i}
            if len(issues) < 100:
                return found
            page += 1

    def locks(self) -> set[int]:
        hours = float(os.environ.get("CI_LOCK_HOURS") or LOCK_HOURS)
        locked = set()
        for number in self.labelled(LOCK_LABEL):
            added = None
            page = 1
            while True:
                events = self.api.get(f"/repos/{self.repo}/issues/{number}/events?per_page=100&page={page}")
                for event in events:
                    if event.get("event") == "labeled" and event.get("label", {}).get("name") == LOCK_LABEL:
                        added = event["created_at"]
                if len(events) < 100:
                    break
                page += 1
            if added and time.time() - stamp(added) < hours * 3600:
                locked.add(number)
        return locked


# The workflows with macOS jobs or gates.
WORKFLOWS = {"ci.yml", "ios.yml", "ios-job.yml", "ios-core.yml", "release.yml"}


def env_int(name: str, default: int) -> int:
    value = os.environ.get(name, "").strip()
    return int(value) if value.isdigit() else default


def summary(text: str) -> None:
    print(text)
    path = os.environ.get("GITHUB_STEP_SUMMARY")
    if path:
        with open(path, "a") as f:
            f.write(text + "\n")


def extra_labels() -> set[str]:
    return {label.strip().lower() for label in os.environ.get("CI_MACOS_LABELS", "").split(",") if label.strip()}


def yield_main() -> int:
    """--yield: should this background job step aside? Errors: go on."""
    repo = os.environ["GITHUB_REPOSITORY"]
    me = int(os.environ["GITHUB_RUN_ID"])
    api = Api(os.environ["GITHUB_TOKEN"], os.environ.get("GITHUB_API_URL", "https://api.github.com"))
    try:
        why = should_yield(Reader(api, repo, extra_labels()).snapshot(me, labels=False))
    except (OSError, RateLimited, KeyError, ValueError) as error:
        print(f"::warning::could not read the macOS runners ({error}); going on")
        return 0
    if why:
        print(f"Stepping aside: {'; '.join(why)}")
        out = os.environ.get("GITHUB_OUTPUT")
        if out:
            with open(out, "a") as f:
                f.write("skip=true\n")
                f.write(f"why={'; '.join(why)}\n")
    else:
        print("Nobody else wants a macOS runner")
    return 0


def main() -> int:
    if "--yield" in sys.argv[1:]:
        return yield_main()
    need = min(env_int("CI_MACOS", 1), MAX_REQUEST)
    cls = os.environ.get("CI_CLASS", "normal") or "normal"
    if cls not in ("normal", "bg", "front"):
        print(f"::warning::unknown class {cls!r}; treated as normal")
        cls = "normal"
    if os.environ.get("CI_ADMISSION", "on").lower() in ("off", "false", "0"):
        summary("macOS admission is off (CI_ADMISSION); going on at once.")
        return 0
    if need <= 0:
        summary("No macOS job to admit; going on at once.")
        return 0
    if cls == "front":
        summary("Main's CI: its release waits for it, so it goes at once (it still counts in everyone's budget).")
        return 0

    budget = env_int("CI_BUDGET_MACOS", 5)
    max_wait = env_int("MAX_WAIT_MINUTES", 180) * 60
    debounce = env_int("CI_DEBOUNCE_SECONDS", 60) if os.environ.get("GITHUB_EVENT_NAME") == "pull_request" else 0
    repo = os.environ["GITHUB_REPOSITORY"]
    me_run = int(os.environ["GITHUB_RUN_ID"])
    api = Api(os.environ["GITHUB_TOKEN"], os.environ.get("GITHUB_API_URL", "https://api.github.com"))
    reader = Reader(api, repo, extra_labels())
    started = time.time()

    if debounce:
        # A newer push to the same pull request cancels this run (ci.yml's concurrency group): pushes in quick
        # succession only take a macOS runner for the last one.
        print(f"Waiting {debounce}s for further pushes")
        time.sleep(debounce)

    print(f"Request: macos={need} ({cls}); budget {budget}")
    failures = 0
    last = None
    while True:
        waited = time.time() - started
        try:
            snap = reader.snapshot(me_run)
            failures = 0
        except RateLimited as error:
            print(f"::warning::{error}")
            if error.wait > 600 or waited + error.wait > max_wait:
                summary("::warning::macOS admission gave up (API rate limit); going on without waiting.")
                return 0
            time.sleep(error.wait)
            continue
        except (OSError, KeyError, ValueError) as error:  # URLError, HTTPError, timeouts, bad JSON
            failures += 1
            print(f"::warning::admission poll failed ({error})")
            if failures >= 5:
                summary("::warning::macOS admission unavailable (API errors); going on without waiting.")
                return 0
            time.sleep(30)
            continue

        decision = decide(own_request(snap, me_run, need, cls), snap, budget)
        state = (decision.position, decision.used, decision.ahead, len(snap.waiters))
        if state != last:
            print(f"[{waited / 60:5.1f} min] position {decision.position}; busy {snap.busy}, reserved "
                  f"{snap.unseen}, releases {snap.release}, needed ahead {decision.ahead}; {len(snap.waiters)} "
                  f"waiting; priority {sorted(snap.priority) or '-'}, locked {sorted(snap.locked) or '-'}; "
                  f"API quota left {api.remaining}")
            for line in snap.lines:
                print(f"    {line}")
            last = state
        if decision.admit:
            summary(f"Admitted after {waited / 60:.1f} min (macos={need} {cls}; busy {snap.busy}, releases "
                    f"{snap.release}, budget {budget}; {api.requests} API requests).")
            return 0
        if waited > max_wait:
            summary(f"::warning::macOS admission waited {waited / 60:.0f} min; going on without waiting.")
            return 0
        interval = 30 if decision.position == 0 else min(300, 60 * decision.position)
        if cls == "bg":
            interval = max(interval, 120)
        if api.remaining < 300:
            interval = max(interval, 300)
        time.sleep(interval)


if __name__ == "__main__":
    sys.exit(main())
